//! PTY-backed tests; the Windows backend arrives in Phase 4.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_node::terminal::{TerminalHandle, TerminalOptions};
use agora_platform::{spawn, PtyConfig};
use agora_vt::Size;

fn handle(argv: &[&str]) -> TerminalHandle {
    let cfg = PtyConfig::new(argv.iter().map(|s| (*s).to_string()).collect());
    TerminalHandle::start(spawn(&cfg).expect("spawn"), TerminalOptions::default()).expect("start")
}

fn wait_for(
    h: &TerminalHandle,
    what: &str,
    mut ok: impl FnMut(&agora_node::terminal::Snapshot) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(10);
    let mut seq = 0;
    while Instant::now() < end {
        let s = h.wait_change(seq, Duration::from_millis(100));
        if ok(&s) {
            return;
        }
        seq = s.seq;
    }
    panic!(
        "timed out waiting for {what}; last screen: {:?}",
        h.snapshot().lines
    );
}

fn fake_agent() -> String {
    // The fake agent binary is built by cargo for this workspace.
    let mut p = std::env::current_exe().expect("exe");
    p.pop(); // deps
    p.pop(); // debug
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    if !p.exists() {
        // `cargo test` only builds binaries of the package under test; build the fake agent on demand.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = std::process::Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success();
        assert!(ok, "building agora-fake-agent failed");
    }
    p.to_string_lossy().into_owned()
}

#[test]
fn fake_agent_screen_is_visible_in_snapshots() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../agora-fake-agent/fixtures/working-blocked-idle.toml"
    );
    let bin = fake_agent();
    let h = handle(&[&bin, "--script", script]);
    wait_for(&h, "blocked prompt", |s| {
        s.lines.iter().any(|l| l.contains("Allow Bash"))
    });
    wait_for(&h, "exit", |s| s.exit.is_some());
    let s = h.snapshot();
    assert!(s.lines.iter().any(|l| l.contains("Done.")));
    assert!(s.exit.expect("exit").success());
}

#[test]
fn input_reaches_the_process_and_resize_is_applied() {
    let h = handle(&["sh"]);
    h.send_input(b"echo hello-$((6*7))\n".to_vec()).unwrap();
    wait_for(&h, "echo result", |s| {
        s.lines.iter().any(|l| l.ends_with("hello-42"))
    });
    h.resize(Size {
        cols: 100,
        rows: 30,
    })
    .unwrap();
    wait_for(&h, "new size", |s| {
        s.size
            == Size {
                cols: 100,
                rows: 30,
            }
    });
    h.kill().unwrap();
    wait_for(&h, "killed", |s| s.exit.is_some());
}

#[test]
fn flooding_terminal_does_not_delay_input_to_another() {
    let flood = handle(&["sh", "-c", "yes 'flood flood flood flood flood flood'"]);
    let calm = handle(&["sh"]);
    wait_for(&flood, "flood output", |s| {
        s.lines.iter().any(|l| l.starts_with("flood"))
    });

    let start = Instant::now();
    calm.send_input(b"echo calm-ok\n".to_vec()).unwrap();
    wait_for(&calm, "calm echo", |s| {
        s.lines.iter().any(|l| l.ends_with("calm-ok"))
    });
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "took {:?}",
        start.elapsed()
    );

    // The flooding terminal itself still accepts input and can be killed promptly.
    flood.kill().unwrap();
    wait_for(&flood, "flood killed", |s| s.exit.is_some());
}

#[test]
fn scrollback_is_capped_and_screen_stays_live() {
    let cfg = PtyConfig::new(vec![
        "sh".into(),
        "-c".into(),
        "seq 1 5000; echo last-line".into(),
    ]);
    let opts = TerminalOptions {
        scrollback: 50,
        ..TerminalOptions::default()
    };
    let h = TerminalHandle::start(spawn(&cfg).unwrap(), opts).unwrap();
    wait_for(&h, "exit", |s| s.exit.is_some());
    let s = h.snapshot();
    assert!(s.lines.iter().any(|l| l.ends_with("last-line")));
    assert!(s.lines.len() <= 24);
}
