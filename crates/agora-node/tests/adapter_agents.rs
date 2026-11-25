//! Agents driven through an L3 adapter (docs/specs/adapter-protocol.md): the supervisor starts the adapter in the
//! agent's own worktree, applies what it reports, and relays prompts and decisions. The adapter is
//! `agora-fake-agent --adapter`; never a real agent. Adapters use pipes, not terminals, so this runs on every OS.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_adapters::host::{AdapterEvent, ChatItem, CutField, ReportedCut};
use agora_adapters::Manifest;
use agora_core::{AgentId, AgentState, NodeId};
use agora_node::chat::ChatBoard;
use agora_node::credentials::{Secret, SecretStore};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;
use agora_node::transcript::{Limits, Store};

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
    // The adapter reported it: it is said to be the agent's own state, not something read from a screen.
    assert!(ev.iter().any(|e| matches!(
        e,
        SupervisorEvent::StateChanged {
            state: AgentState::Working,
            reported: true,
            ..
        }
    )));
    assert!(s.list().iter().any(|v| v.id == id && v.reported));
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
    // ADR-0090: nor does it take a config option, and it never reported its turns (`turns`), so a prompt is sent at once and never
    // queued, however many arrive: an adapter that never says `idle` must not hold prompts back for good.
    assert_eq!(
        s.adapter_set_config(&id, "effort", "low"),
        Err(agora_node::supervisor::SupervisorError::OptionUnsupported(
            "agent.set_config"
        ))
    );
    for _ in 0..3 {
        match s.adapter_submit(&id, "hello", |_| panic!("queued")) {
            Ok(agora_node::supervisor::Submit::Send(call)) => call.run().unwrap(),
            _ => panic!("a prompt to an adapter without `turns` is sent at once"),
        }
    }
}

/// ADR-0090: the supervisor's lock is not held while an adapter answers. An adapter that never answers a prompt stalls only the call
/// made to it: the tick, a listing, and another agent's commands go on answering while it hangs.
#[test]
fn a_hung_adapter_stalls_only_its_own_call_not_the_supervisor() {
    use std::sync::{Arc, Mutex};
    let e = env("hung");
    let mut s = supervisor();
    let hung = start(&mut s, &e, "hung");
    let other = start(&mut s, &e, "other");
    until(&mut s, state_is(AgentState::Idle));
    settle(&mut s, &hung, AgentState::Idle);
    settle(&mut s, &other, AgentState::Idle);
    let sup = Arc::new(Mutex::new(s));
    // The call is prepared under the lock and made after it is let go, as the node does it.
    let call = sup
        .lock()
        .unwrap()
        .prepare_prompt(&hung, "hang", false)
        .unwrap();
    let asked = Instant::now();
    let stuck = std::thread::spawn(move || call.run());
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !stuck.is_finished(),
        "the adapter has not answered: the call is still waiting"
    );
    let begun = Instant::now();
    // Everything else answers at once: a listing, a tick, the other agent's prompt, a refused option, a mode switch's check.
    let views = sup.lock().unwrap().list();
    assert_eq!(views.len(), 2);
    sup.lock().unwrap().tick(Instant::now());
    let call = sup
        .lock()
        .unwrap()
        .prepare_prompt(&other, "hello", false)
        .unwrap();
    call.run().unwrap();
    assert!(sup
        .lock()
        .unwrap()
        .prepare_set_config(&other, "effort", "low")
        .is_err());
    assert!(
        begun.elapsed() < Duration::from_secs(3),
        "the lock was held while the adapter hung: {:?}",
        begun.elapsed()
    );
    assert!(!stuck.is_finished());
    // The hung call ends by its own timeout (10 s), with an error, and nothing else was waiting on it. #1150: a prompt that was not
    // answered in time is a failed prompt, never a failed start (`agent start failed`, reference `unassigned`).
    let failed = stuck.join().expect("the call's thread");
    assert!(
        asked.elapsed() >= Duration::from_secs(9),
        "the call ended before its timeout: {:?}",
        asked.elapsed()
    );
    drop(sup);
    let Err(e) = failed else {
        panic!("the hung prompt was answered: {failed:?}")
    };
    // ADR-0110: the caller is told the adapter did not take the prompt in time (`adapter.no_answer`), never `internal`.
    no_answer(&e, "prompt");
}

/// ADR-0110: `e` is the stable answer to a call the adapter did not answer within the node's 10 s wait.
fn no_answer(e: &agora_node::supervisor::SupervisorError, call: &str) {
    use agora_protocol::ParamValue;
    let answer = e.to_protocol();
    assert_eq!(
        answer.code,
        agora_protocol::ErrorCode::AdapterNoAnswer,
        "{e:?}"
    );
    let want = [
        ("call".to_string(), ParamValue::Str(call.into())),
        ("kind".to_string(), ParamValue::Str("fake-adapter".into())),
        ("waited_ms".to_string(), ParamValue::Int(10_000)),
    ];
    assert_eq!(answer.params, want.into_iter().collect(), "{e:?}");
}

/// ADR-0110: `e` passes on the adapter's `adapter.agent_exited` (from `agora-fake-agent --adapter`'s `prompt-exited` and
/// `ask-exited`) with only that code's parameters, as strings cut at 600 bytes, as a failed start does.
fn passed_on_exit(e: &agora_node::supervisor::SupervisorError) {
    use agora_protocol::ParamValue;
    let answer = e.to_protocol();
    assert_eq!(
        answer.code,
        agora_protocol::ErrorCode::AdapterAgentExited,
        "{e:?}"
    );
    let want = [
        ("exit_code".to_string(), ParamValue::Str("3".into())),
        ("stage".to_string(), ParamValue::Str("turn".into())),
        ("stderr_tail".to_string(), ParamValue::Str("x".repeat(600))),
    ];
    assert_eq!(answer.params, want.into_iter().collect(), "{e:?}");
}

/// `e` is still `internal`, with the reference that names the call (#1150).
fn still_internal(e: &agora_node::supervisor::SupervisorError, incident: &str) {
    let answer = e.to_protocol();
    assert_eq!(answer.code, agora_protocol::ErrorCode::Internal, "{e:?}");
    assert_eq!(
        answer.params.get("incident_id"),
        Some(&agora_protocol::ParamValue::Str(incident.into()))
    );
}

#[test]
fn a_prompt_the_adapter_refuses_because_its_agent_exited_is_passed_on() {
    let e = env("prompt-exited");
    let mut s = supervisor();
    let id = start(&mut s, &e, "pe");
    until(&mut s, state_is(AgentState::Idle));
    let err = s
        .adapter_prompt(&id, "prompt-exited", false)
        .expect_err("the adapter refused the prompt");
    passed_on_exit(&err);
}

#[test]
fn a_prompt_the_adapter_refuses_without_a_stable_reason_stays_internal() {
    let e = env("prompt-error");
    let mut s = supervisor();
    let id = start(&mut s, &e, "pr");
    until(&mut s, state_is(AgentState::Idle));
    let err = s
        .adapter_prompt(&id, "prompt-error", false)
        .expect_err("the adapter refused the prompt");
    still_internal(&err, "agent.prompt");
}

/// Starts an adapter agent and sends it `prompt` (one of the fake adapter's `ask` prompts), then waits for its request `ap-1`.
fn asking(name: &str, prompt: &str) -> (Env, Supervisor, AgentId) {
    let e = env(name);
    let mut s = supervisor();
    let id = start(&mut s, &e, name);
    until(&mut s, state_is(AgentState::Idle));
    s.adapter_prompt(&id, prompt, false).unwrap();
    let ev = until(&mut s, |e| {
        matches!(
            e,
            SupervisorEvent::Adapter {
                event: AdapterEvent::ApprovalRequest { .. },
                ..
            }
        )
    });
    assert!(
        matches!(ev.last(), Some(SupervisorEvent::Adapter { event: AdapterEvent::ApprovalRequest { approval_id, .. }, .. }) if approval_id == "ap-1")
    );
    (e, s, id)
}

#[test]
fn an_approval_answer_the_adapter_does_not_take_in_time_is_no_answer() {
    let (_e, s, id) = asking("ask-hang", "ask-hang");
    let asked = Instant::now();
    let err = s
        .adapter_resolve_approval(&id, "ap-1", true)
        .expect_err("the adapter did not take the answer");
    assert!(
        asked.elapsed() >= Duration::from_secs(9),
        "the call ended before its timeout: {:?}",
        asked.elapsed()
    );
    no_answer(&err, "approval");
}

#[test]
fn an_approval_answer_the_adapter_refuses_because_its_agent_exited_is_passed_on() {
    let (_e, s, id) = asking("ask-exited", "ask-exited");
    let err = s
        .adapter_resolve_approval(&id, "ap-1", true)
        .expect_err("the adapter refused the answer");
    passed_on_exit(&err);
}

#[test]
fn an_approval_answer_the_adapter_refuses_without_a_stable_reason_stays_internal() {
    let (_e, s, id) = asking("ask-error", "ask-error");
    let err = s
        .adapter_resolve_approval(&id, "ap-1", true)
        .expect_err("the adapter refused the answer");
    still_internal(&err, "approval.resolve");
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
    // The node's own fact, not the adapter's report.
    assert!(ev.iter().any(|e| matches!(
        e,
        SupervisorEvent::StateChanged {
            state: AgentState::Unknown,
            reported: false,
            ..
        }
    )));
    let list = s.list();
    assert!(!list.iter().find(|v| v.id == id).unwrap().reported);
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
    // #902: a person's stop is a stop, never a give-up with an exit (in the stop's answer or the ticks after it).
    std::thread::sleep(Duration::from_millis(200));
    let after = s.tick(Instant::now());
    assert!(
        !ev.iter().chain(after.iter()).any(|e| matches!(
            e,
            SupervisorEvent::Exited { .. } | SupervisorEvent::GaveUp { .. }
        )),
        "{ev:#?} {after:#?}"
    );
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
    let ev = until(&mut s, |e| matches!(e, SupervisorEvent::GaveUp { .. }));
    assert!(!s.list().iter().find(|v| v.id == id).unwrap().running);
    // #902 (RM-13): giving up ends the agent with the exit code its adapter crashed with (the fake adapter's `crash` exits 3), right
    // before `GaveUp`, so a window shows it ended with an error instead of keeping it unknown.
    let n = ev.len();
    assert!(n >= 2, "{ev:#?}");
    assert_eq!(
        ev[n - 2],
        SupervisorEvent::Exited {
            agent: id.clone(),
            status: agora_platform::ExitStatus {
                code: Some(3),
                signal: None
            },
        },
        "{ev:#?}"
    );
    assert_eq!(
        ev[n - 1],
        SupervisorEvent::GaveUp {
            agent: id.clone(),
            exit_code: Some(3),
        }
    );
    // Only the end is reported as exited: the earlier crashes were restarts.
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, SupervisorEvent::Exited { .. }))
            .count(),
        1
    );
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
    let stopped = s.stop(&id).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let ev = s.tick(std::time::Instant::now());
    assert!(!ev
        .iter()
        .any(|e| matches!(e, SupervisorEvent::Started { .. })));
    // #902: stopped while a restart waited: no give-up and no exit is reported for it.
    assert!(!ev.iter().chain(stopped.iter()).any(|e| matches!(
        e,
        SupervisorEvent::Exited { .. } | SupervisorEvent::GaveUp { .. }
    )));
    assert!(!s.list().iter().find(|v| v.id == id).unwrap().running);
}

/// ADR-0090 Amendment A: a switch the owner approved names one run of the agent's adapter. After the adapter restarted (a crash), the same
/// approval applies nothing, even though the new run reports the same mode; a request made in the new run applies.
#[test]
fn an_approval_for_one_run_of_an_adapter_is_not_applied_to_the_next() {
    use agora_adapters::host::OptionKind;
    use agora_node::supervisor::SupervisorError;
    let e = env("epoch");
    let mut s = supervisor_with(RestartPolicy {
        max_restarts: 2,
        base_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(100),
    });
    s.register(
        Manifest::parse(&format!(
            "kind = \"modal\"\ndisplay_name = \"M\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\", \"--modes\"]\n",
            fake_agent()
        ))
        .unwrap(),
    );
    let id = s
        .start(
            &StartRequest {
                kind: "modal".into(),
                checkout: e.repo.clone(),
                worktree_root: e.dir.join("worktrees"),
                name: "modal".into(),
                base: None,
            },
            Instant::now(),
        )
        .expect("start");
    // The modes arrive just after the start.
    let reported = |s: &mut Supervisor| {
        let end = Instant::now() + Duration::from_secs(15);
        while s.adapter_set_option(&id, OptionKind::Mode, "default") != Ok(None) {
            assert!(Instant::now() < end, "the modes were never reported");
            s.tick(Instant::now());
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    reported(&mut s);
    let ask = s
        .adapter_set_option(&id, OptionKind::Mode, "bypassPermissions")
        .unwrap()
        .expect("a mode that skips is asked about, not applied");
    assert_eq!(
        (ask.value.as_str(), ask.option_id.as_str()),
        ("bypassPermissions", "mode")
    );
    assert!(
        s.prepare_skip_apply(&ask).is_ok(),
        "the same run applies it"
    );

    let _ = s.adapter_prompt(&id, "crash", false);
    until(&mut s, |e| matches!(e, SupervisorEvent::Started { .. }));
    reported(&mut s);
    assert!(
        matches!(
            s.prepare_skip_apply(&ask),
            Err(SupervisorError::NotFound(_))
        ),
        "an approval for the last run applies nothing in this one"
    );
    let fresh = s
        .adapter_set_option(&id, OptionKind::Mode, "bypassPermissions")
        .unwrap()
        .unwrap();
    assert!(s.prepare_skip_apply(&fresh).is_ok());
}

/// ADR-0082 decision 4 and ADR-0084 (audit F840): an adapter agent whose confinement reaches the control token file, or the folder of
/// stored secrets once it holds one, is refused at the start, as a terminal agent is. The adapter and the agent below it run with that
/// profile, so starting it would hand them the token (and the node's control socket) or every stored secret.
#[test]
fn a_confined_adapter_that_could_read_the_control_token_or_the_secrets_is_refused() {
    use agora_node::credentials::{Credentials, Secret, SecretStore};
    use agora_node::sandbox::{Level, SandboxPolicy};
    use agora_node::supervisor::SupervisorError;
    if !agora_platform::sandbox_supported() {
        return;
    }
    private_home::init();
    // Under the target folder, not the temp folder a confined agent may write on Linux.
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("adp-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (repo, node, data) = (root.join("repo"), root.join("node"), root.join("data"));
    for d in [&repo, &node, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let token = node.join("control.token");
    std::fs::write(&token, "t").unwrap();
    let e = Env {
        dir: root.clone(),
        repo,
    };
    let start_with = |read: Vec<PathBuf>, name: &str| {
        let mut s = supervisor();
        s.set_sandbox(SandboxPolicy {
            level: Level::Worktree,
            read,
            control_token: Some(token.clone()),
            protect: vec![node.clone(), data.clone()],
            ..SandboxPolicy::default()
        });
        s.set_credentials(Credentials::new(
            SecretStore::in_data(&data),
            Default::default(),
        ));
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
        .map(|_| ())
    };
    let refused = |r: Result<(), SupervisorError>, why: &str| matches!(r, Err(SupervisorError::SandboxRefused(_, _, w)) if w == why);
    // Nothing reaches the token: the adapter starts.
    start_with(vec![], "fine").unwrap();
    // The owner's read grant covers the token's folder: refused, and nothing is left behind.
    assert!(refused(
        start_with(vec![node.clone()], "token"),
        "control_token_reachable"
    ));
    assert!(!root.join("worktrees/token").exists());
    // The data folder once it holds a secret.
    SecretStore::in_data(&data)
        .set("ADP_TOKEN", &Secret::new("v").unwrap())
        .unwrap();
    assert!(refused(
        start_with(vec![data.clone()], "secrets"),
        "secrets_reachable"
    ));
    let _ = std::fs::remove_dir_all(&root);
}

/// A supervisor whose fake adapter kind has a launch option that makes it act without asking (its argument is one the fake ignores).
fn supervisor_with_skip_option(restart: RestartPolicy) -> Supervisor {
    let mut s = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart,
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    s.register(
        Manifest::parse(&format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 2\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n\
             [[launch.option]]\nid = \"skip\"\nlabel = \"option-skip\"\nargs = [\"--yes\"]\nskips_permissions = true\n",
            fake_agent()
        ))
        .unwrap(),
    );
    s
}

fn launch_of(s: &Supervisor, id: &AgentId) -> (Vec<String>, bool) {
    let v = s.list().into_iter().find(|v| &v.id == id).expect("listed");
    (v.launch_options, v.skips_permissions)
}

/// Independent review of #928, point 3: only an agent taken up from the holder (`set_kept_launch`, whose options' arguments are not
/// known again) loses its launch options at a crash restart, since its next process runs without them. An agent this node started with
/// a skipping option keeps the option and the badge across a crash restart (its arguments are applied again). A manifest option always
/// has arguments, so the old test of "no arguments" could not tell the two apart by itself; the explicit kept set does.
#[test]
fn only_an_adapter_taken_up_from_the_holder_loses_its_launch_options_at_a_crash_restart() {
    let e = env("keptlaunch");
    let policy = RestartPolicy {
        max_restarts: 3,
        base_delay: Duration::from_millis(50),
        max_delay: Duration::from_millis(100),
    };
    let mut s = supervisor_with_skip_option(policy);
    let skip = agora_node::launch_options::Requested {
        ids: vec!["skip".into()],
        confirmed: vec!["skip".into()],
        settings: Vec::new(),
    };
    let req = |name: &str| StartRequest {
        kind: "fake-adapter".into(),
        checkout: e.repo.clone(),
        worktree_root: e.dir.join("worktrees"),
        name: name.into(),
        base: None,
    };

    // Started here with the option: it survives a crash restart.
    let own = s
        .start_with_options(&req("own"), &skip, Instant::now())
        .expect("start");
    settle(&mut s, &own, AgentState::Idle);
    assert_eq!(launch_of(&s, &own), (vec!["skip".to_string()], true));
    let _ = s.adapter_prompt(&own, "crash", false);
    until(
        &mut s,
        |e| matches!(e, SupervisorEvent::Started { agent } if *agent == own),
    );
    settle(&mut s, &own, AgentState::Idle);
    assert_eq!(
        launch_of(&s, &own),
        (vec!["skip".to_string()], true),
        "an agent started here lost its launch options at a crash restart"
    );
    assert!(s.skips_permissions_now(&own));

    // Its options put back from the ledger as at a take-up: the restarted process runs without their arguments, so it is shown without.
    let kept = start(&mut s, &e, "kept");
    settle(&mut s, &kept, AgentState::Idle);
    s.set_kept_launch(&kept, &["skip".to_string()], true);
    assert_eq!(launch_of(&s, &kept), (vec!["skip".to_string()], true));
    assert!(s.skips_permissions_now(&kept));
    let _ = s.adapter_prompt(&kept, "crash", false);
    until(
        &mut s,
        |e| matches!(e, SupervisorEvent::Started { agent } if *agent == kept),
    );
    settle(&mut s, &kept, AgentState::Idle);
    assert_eq!(
        launch_of(&s, &kept),
        (Vec::new(), false),
        "a kept agent's next process was shown with options it does not run with"
    );
    assert!(!s.skips_permissions_now(&kept));
    // The agent started here is untouched by the other's restart.
    assert_eq!(launch_of(&s, &own), (vec!["skip".to_string()], true));
    let _ = std::fs::remove_dir_all(&e.dir);
}

/// ADR-0111: a supervisor whose fake adapter declares `chat.cuts` (`--cuts`) or not.
fn supervisor_cuts(declare: bool) -> Supervisor {
    let mut s = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy {
            max_restarts: 0,
            ..RestartPolicy::default()
        },
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    let extra = if declare { ", \"--cuts\"" } else { "" };
    s.register(
        Manifest::parse(&format!(
            "kind = \"fake-adapter\"\ndisplay_name = \"F\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\"{extra}]\n",
            fake_agent()
        ))
        .unwrap(),
    );
    s
}

/// ADR-0111: a value the adapter cut in the middle (letters only, so nothing else the file holds matches a piece of it).
const CUT_VALUE: &str = "sk-CUTS-qwertyuiopASDFGHJKLzxcvbnmPOIUYT";

/// The pieces of `v` of 4 bytes from each character (finding none finds no longer one).
fn pieces_of(v: &str) -> Vec<&str> {
    (0..v.len().saturating_sub(3))
        .filter(|&i| v.is_char_boundary(i) && v.is_char_boundary(i + 4))
        .map(|i| &v[i..i + 4])
        .collect()
}

/// The adapter's tool call for `prompt` (`cut:` or `cut-unreported:`), with the places the supervisor passed on; and what a window
/// following the agent and the saved transcript hold of [`CUT_VALUE`] once the node keeps it.
fn cut_through_node(name: &str, declare: bool, prompt: &str) -> (Vec<ReportedCut>, Option<String>) {
    let e = env(name);
    let mut s = supervisor_cuts(declare);
    let id = start(&mut s, &e, name);
    settle(&mut s, &id, AgentState::Idle);
    s.adapter_prompt(
        &id,
        &format!("{prompt}the key is {CUT_VALUE} and more words"),
        false,
    )
    .unwrap();
    let mut got = None;
    until(&mut s, |ev| match ev {
        SupervisorEvent::Adapter {
            event:
                AdapterEvent::Chat {
                    item: item @ ChatItem::Tool(_),
                    cuts,
                    ..
                },
            ..
        } => {
            got = Some((item.clone(), cuts.clone()));
            true
        }
        _ => false,
    });
    let (item, cuts) = got.unwrap();
    // The node keeps it as the daemon does (`chat_event`), with the value in its secret store.
    let data = e.dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    SecretStore::in_data(&data)
        .set("TOKEN", &Secret::new(CUT_VALUE).unwrap())
        .unwrap();
    let store = Arc::new(Store::new(
        &data,
        Limits::default(),
        Some(SecretStore::in_data(&data)),
    ));
    let board = ChatBoard::saving(Arc::clone(&store));
    let (_, sub) = board.follow(&id);
    board.push_item_cut(&id, &item, &cuts, 1);
    for batch in board.take_unsaved() {
        store.write(&batch).unwrap();
    }
    let mut seen = Vec::new();
    while let Ok(entry) = sub.recv_timeout(Duration::from_millis(200)) {
        seen.push(entry.output);
    }
    seen.extend(
        store
            .read(id.as_str())
            .entries
            .into_iter()
            .map(|(e, _)| e.output),
    );
    assert!(
        seen.iter().any(|o| o.ends_with("and more words")),
        "{seen:?}"
    );
    let leak = pieces_of(CUT_VALUE)
        .into_iter()
        .find(|p| seen.iter().any(|o| o.contains(p)))
        .map(|p| format!("{p:?} in {seen:?}"));
    let _ = s.stop(&id);
    let _ = std::fs::remove_dir_all(&e.dir);
    (cuts, leak)
}

/// ADR-0111: the fake adapter cuts a held value in the middle of a tool's output and reports the cut: the node keeps the place, and
/// neither what a window is sent nor the saved transcript holds a piece of the value (4 bytes or more).
#[test]
fn a_cut_an_adapter_reports_reaches_the_node_and_no_piece_of_the_value_is_kept() {
    let (cuts, leak) = cut_through_node("cuts-reported", true, "cut:");
    assert_eq!(cuts.len(), 1, "{cuts:?}");
    assert_eq!(cuts[0].field, CutField::Output);
    assert_eq!(leak, None);
}

/// ADR-0111: an adapter that did not declare `chat.cuts` is read as before, whatever it sends: its places are not used, and the
/// value's pieces beside the cut stay (the residual T61 keeps for such adapters). One that declared it but did not report a cut
/// leaves them too: the contract is the adapter's.
#[test]
fn an_adapter_without_the_capability_or_without_the_report_is_read_as_before() {
    let (cuts, leak) = cut_through_node("cuts-undeclared", false, "cut:");
    assert!(cuts.is_empty(), "{cuts:?}");
    assert!(leak.is_some(), "a piece beside an unreported cut");
    let (cuts, leak) = cut_through_node("cuts-unreported", true, "cut-unreported:");
    assert!(cuts.is_empty(), "{cuts:?}");
    assert!(leak.is_some(), "a piece beside an unreported cut");
}
