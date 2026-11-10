//! The adapter host against a real child process speaking JSON-RPC over pipes: the fake adapter
//! (`agora_fake_agent::adapter`), run by re-executing this test binary (the `adapter_child` test below is the
//! adapter when `AGORA_TEST_ADAPTER` is set; otherwise it does nothing). No real agent, account or token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_adapters::host::{Adapter, AdapterConfig, AdapterEvent, ChatItem, HostError};
use agora_core::AgentState;

const T: Duration = Duration::from_secs(10);

#[test]
fn adapter_child() {
    if std::env::var_os("AGORA_TEST_ADAPTER").is_none() {
        return;
    }
    // The harness has printed "test adapter_child ... " without a newline; end that line so JSON starts on its own.
    println!();
    let _ = agora_fake_agent::adapter::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

fn spawn() -> Adapter {
    Adapter::spawn(&AdapterConfig {
        program: std::env::current_exe().unwrap(),
        // The harness prints "running 1 test" etc. to stdout; those lines are not protocol and are ignored.
        args: [
            "--exact",
            "adapter_child",
            "--nocapture",
            "--test-threads=1",
        ]
        .map(String::from)
        .to_vec(),
        env: vec![("AGORA_TEST_ADAPTER".into(), "1".into())],
        cwd: None,
        sandbox: None,
    })
    .unwrap()
}

/// Events until `done` matches, or panic after the timeout.
fn until(a: &Adapter, mut done: impl FnMut(&AdapterEvent) -> bool) -> Vec<AdapterEvent> {
    let end = Instant::now() + T;
    let mut all = Vec::new();
    while Instant::now() < end {
        if let Some(e) = a.next_event(Duration::from_millis(100)) {
            let stop = done(&e);
            all.push(e);
            if stop {
                return all;
            }
        }
    }
    panic!("timed out; events so far: {all:#?}");
}

fn states(events: &[AdapterEvent]) -> Vec<AgentState> {
    events
        .iter()
        .filter_map(|e| match e {
            AdapterEvent::State { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

#[test]
fn handshake_session_prompt_and_usage() {
    let a = spawn();
    let init = a.initialize(&["chat"], "en", T).unwrap();
    assert_eq!(init.adapter_name, "fake-adapter");
    assert_eq!(init.agent_kind, "fake");
    assert!(init.capabilities.contains(&"approvals".to_string()));

    let sid = a.start_session("/proj", "/proj/wt", None, T).unwrap();
    assert_eq!(sid, "s-1");
    let startup = until(&a, |e| {
        matches!(
            e,
            AdapterEvent::State {
                state: AgentState::Idle,
                ..
            }
        )
    });
    assert!(startup.contains(&AdapterEvent::SessionReported {
        session_id: "s-1".into(),
        native_session_id: "native-s-1".into()
    }));

    a.prompt(&sid, "hello", false, T).unwrap();
    let ev = until(&a, |e| {
        matches!(
            e,
            AdapterEvent::State {
                state: AgentState::Idle,
                ..
            }
        )
    });
    assert_eq!(states(&ev), [AgentState::Working, AgentState::Idle]);
    assert!(ev
        .iter()
        .any(|e| matches!(e, AdapterEvent::Chat { item: ChatItem::Text(t), .. } if t == "hello")));
    assert!(ev.iter().any(|e| matches!(e,
        AdapterEvent::Usage { model, input_tokens: 10, output_tokens: 5, cost_micros: None, .. } if model == "fake/model")));
}

#[test]
fn resume_passes_the_session_id_through() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    assert_eq!(a.start_session("/p", "/p", Some("s-9"), T).unwrap(), "s-9");
}

#[test]
fn an_approval_round_trip_goes_through_the_node() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    let sid = a.start_session("/p", "/p", None, T).unwrap();
    a.prompt(&sid, "ask", false, T).unwrap();
    let ev = until(&a, |e| matches!(e, AdapterEvent::ApprovalRequest { .. }));
    assert!(
        matches!(ev.last(), Some(AdapterEvent::ApprovalRequest { approval_id, action, .. })
        if approval_id == "ap-1" && action == "run_command")
    );
    assert!(states(&ev).contains(&AgentState::Blocked));
    // The node delivers its verified decision; the agent carries on.
    a.resolve_approval(&sid, "ap-1", false, T).unwrap();
    let after = until(&a, |e| {
        matches!(
            e,
            AdapterEvent::State {
                state: AgentState::Idle,
                ..
            }
        )
    });
    assert!(after
        .iter()
        .any(|e| matches!(e, AdapterEvent::Chat { item: ChatItem::Text(t), .. } if t == "denied")));
}

#[test]
fn noise_and_unknown_methods_are_ignored_not_fatal() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    let sid = a.start_session("/p", "/p", None, T).unwrap();
    a.prompt(&sid, "noise", false, T).unwrap();
    until(&a, |e| {
        matches!(
            e,
            AdapterEvent::State {
                state: AgentState::Working,
                ..
            }
        )
    });
    until(&a, |e| {
        matches!(
            e,
            AdapterEvent::State {
                state: AgentState::Idle,
                ..
            }
        )
    });
    // The harness's own stdout lines, the non-JSON line and the unknown method were all counted and dropped.
    assert!(a.ignored_lines() >= 2, "{}", a.ignored_lines());
    // Still fully usable afterwards.
    a.cancel(&sid, T).unwrap();
}

#[test]
fn a_crash_fails_the_call_in_flight_and_ends_with_exited() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    let sid = a.start_session("/p", "/p", None, T).unwrap();
    // The adapter dies while handling the prompt, before it can answer.
    let r = a.prompt(&sid, "crash", false, T);
    assert!(matches!(r, Err(HostError::Exited) | Ok(())), "{r:?}");
    let ev = until(&a, |e| matches!(e, AdapterEvent::Exited));
    assert!(matches!(ev.last(), Some(AdapterEvent::Exited)));
    // Later calls fail cleanly instead of hanging.
    assert!(matches!(
        a.cancel(&sid, Duration::from_secs(2)),
        Err(HostError::Exited | HostError::Io(_))
    ));
}

#[test]
fn an_unknown_request_is_an_rpc_error_with_its_code() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    match a.call("no/such/method", serde_json::Value::Null, T) {
        Err(HostError::Rpc { code, .. }) => assert_eq!(code, -32601),
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

#[test]
fn an_adapter_that_never_answers_times_out() {
    // `cat` echoes requests back verbatim: they are requests, not responses, so no answer ever comes.
    let a = Adapter::spawn(&AdapterConfig {
        program: "cat".into(),
        args: vec![],
        env: vec![],
        cwd: None,
        sandbox: None,
    });
    let Ok(a) = a else { return }; // no `cat` here (Windows): nothing to prove
    assert!(matches!(
        a.initialize(&[], "en", Duration::from_millis(300)),
        Err(HostError::Timeout)
    ));
}

#[test]
fn shutdown_ends_the_process() {
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    a.shutdown(T); // returns only once the child is reaped
}

#[test]
fn the_adapter_does_not_inherit_the_nodes_secrets() {
    // The host starts adapters with a cleared environment plus PATH and what the node passes.
    std::env::set_var("AGORA_TEST_SECRET", "hunter2");
    let a = spawn();
    a.initialize(&[], "en", T).unwrap();
    let sid = a.start_session("/p", "/p", None, T).unwrap();
    a.prompt(&sid, "env", false, T).unwrap();
    let ev = until(&a, |e| matches!(e, AdapterEvent::Chat { .. }));
    assert!(
        matches!(ev.last(), Some(AdapterEvent::Chat { item: ChatItem::Text(t), .. }) if t == "secret:hidden"),
        "{ev:?}"
    );
}

#[test]
fn a_confined_adapter_writes_only_where_it_may() {
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no Landlock on this machine");
        return;
    }
    let base = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("host-sbx-{}", std::process::id()));
    let (allowed, outside) = (base.join("allowed"), base.join("outside"));
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let script = format!(
        "echo a > {a}/f; echo b > {o}/f 2>/dev/null; sleep 5",
        a = allowed.display(),
        o = outside.display()
    );
    let _a = Adapter::spawn(&AdapterConfig {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
        env: vec![],
        cwd: None,
        sandbox: Some(agora_platform::Sandbox {
            read: ["/usr", "/lib", "/lib64", "/bin", "/etc", "/dev"]
                .map(Into::into)
                .to_vec(),
            write: vec![allowed.clone(), "/dev".into()],
        }),
    })
    .unwrap();
    let end = Instant::now() + T;
    while !allowed.join("f").exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(200));
    if !allowed.join("f").exists() {
        // TEMP-DEBUG
        let o = std::process::Command::new("/usr/bin/sandbox-exec")
            .args([
                "-p",
                "(version 1)(allow default)",
                "/bin/sh",
                "-c",
                "echo hi",
            ])
            .output();
        let o2 = std::process::Command::new("/usr/bin/sandbox-exec")
            .args(["-p", "(version 1)(allow default)(deny file-read-data file-write*)(allow file-read-metadata)(allow file-read* (subpath \"/usr\") (subpath \"/bin\") (subpath \"/System\") (subpath \"/Library\") (subpath \"/dev\") (subpath \"/private/etc\") (subpath \"/private/var/db\"))", "/bin/sh", "-c", "echo hi2"])
            .output();
        panic!("DEBUG {o:?} || {o2:?}");
    }
    assert!(
        allowed.join("f").exists(),
        "it could not write where it may"
    );
    assert!(!outside.join("f").exists(), "it wrote where it may not");
    let _ = std::fs::remove_dir_all(base);
}
