//! `spawn_bound`: a child ends when its supervisor ends, even when the supervisor is killed outright (ADR-0062 item 8). The
//! supervisor and the child are this test binary again, each running one helper below.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const ROLE: &str = "AGORA_BOUND_TEST_ROLE";

fn helper(name: &str, role: &str) -> Command {
    let mut c = Command::new(std::env::current_exe().unwrap());
    c.args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(ROLE, role);
    c
}

/// Not a test: the supervisor. Starts the child bound to it, prints the child's id and whether it is bound, then waits to be killed.
#[test]
fn supervisor_helper() {
    if std::env::var(ROLE).as_deref() != Ok("supervisor") {
        return;
    }
    let mut cmd = helper("child_helper", "child");
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    let b = agora_platform::spawn_bound(&mut cmd).unwrap();
    println!("child {} {}", b.child.id(), b.bound);
    std::thread::sleep(Duration::from_secs(120));
}

/// Not a test: the child. Ends with its supervisor (Unix needs this call; Windows does it by the job), else lives two minutes.
#[test]
fn child_helper() {
    if std::env::var(ROLE).as_deref() != Ok("child") {
        return;
    }
    agora_platform::end_with_supervisor();
    assert!(
        std::env::var_os("AGORA_SUPERVISOR_PID").is_none(),
        "the variable stays for the child's own children"
    );
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn killing_the_supervisor_outright_ends_its_bound_child() {
    let mut supervisor = helper("supervisor_helper", "supervisor")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(supervisor.stdout.take().unwrap());
    // libtest prints "test supervisor_helper ... " on the same line first.
    while !line.contains("child ") {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "no child reported");
    }
    let at = line.find("child ").unwrap();
    let mut w = line[at..].split_whitespace().skip(1);
    let child: u32 = w.next().unwrap().parse().unwrap();
    let bound: bool = w.next().unwrap().parse().unwrap();
    if !bound {
        // A system without a parent-exit watch: nothing to check.
        let _ = supervisor.kill();
        let _ = supervisor.wait();
        return;
    }
    // Give a Unix child time to set its watch before its supervisor goes.
    std::thread::sleep(Duration::from_millis(500));
    assert!(agora_platform::process_start_time(child).is_some());
    // SIGKILL on Unix, TerminateProcess on Windows: the supervisor runs no code of its own on the way out.
    supervisor.kill().unwrap();
    supervisor.wait().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while agora_platform::process_start_time(child).is_some() {
        assert!(
            Instant::now() < end,
            "the child {child} outlived its killed supervisor"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
