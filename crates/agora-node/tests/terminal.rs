//! PTY-backed tests, on every platform with a pseudo-terminal (ADR-0045).
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

use agora_node::terminal::{Output, OutputSubscription};
use agora_vt::{AlacrittyTerminal, Terminal};

/// Apply subscription items to a fresh terminal until `done` says the screen is as expected.
fn drain(
    sub: &OutputSubscription,
    term: &mut AlacrittyTerminal,
    mut done: impl FnMut(&AlacrittyTerminal) -> bool,
) {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        match sub.recv_timeout(Duration::from_millis(100)) {
            Ok(Output::Data(b)) => term.feed(&b),
            Ok(Output::Resize(s)) => term.resize(s),
            Ok(Output::Exited(_)) | Err(_) => {}
            Ok(Output::Lagged) => panic!("unexpected lag"),
        }
        if done(term) {
            return;
        }
    }
    panic!("timed out; screen: {:?}", term.bottom_lines(24));
}

fn lines(t: &dyn Terminal) -> Vec<String> {
    (0..t.size().rows).map(|r| t.line(r)).collect()
}

#[test]
fn snapshot_plus_subscription_reconstructs_the_live_screen_exactly() {
    let h = handle(&["sh"]);
    h.send_input(b"echo before-subscribe\n".to_vec()).unwrap();
    wait_for(&h, "first echo", |s| {
        s.lines.iter().any(|l| l.ends_with("before-subscribe"))
    });

    let (snap, sub) = h.subscribe();
    // A client builds its view from the snapshot lines, then applies live output after it.
    let mut mirror = AlacrittyTerminal::new(snap.size);
    for (i, l) in snap.lines.iter().enumerate() {
        mirror.feed(format!("\x1b[{};1H{l}", i + 1).as_bytes());
    }
    // The cursor matters: the shell positions later output relative to it.
    mirror.feed(format!("\x1b[{};{}H", snap.cursor.row + 1, snap.cursor.col + 1).as_bytes());
    h.send_input(b"echo after-subscribe\n".to_vec()).unwrap();
    drain(&sub, &mut mirror, |t| {
        lines(t).iter().any(|l| l.ends_with("after-subscribe"))
    });

    // Same screen as the node's own view (once it has caught up).
    wait_for(&h, "settled", |s| {
        s.lines.iter().any(|l| l.ends_with("after-subscribe"))
    });
    let live = h.snapshot().lines;
    let end = Instant::now() + Duration::from_secs(5);
    while lines(&mirror) != live && Instant::now() < end {
        drain_once(&sub, &mut mirror);
    }
    assert_eq!(lines(&mirror), live);
    h.kill().unwrap();
}

fn drain_once(sub: &OutputSubscription, term: &mut AlacrittyTerminal) {
    if let Ok(Output::Data(b)) = sub.recv_timeout(Duration::from_millis(50)) {
        term.feed(&b);
    }
}

#[test]
fn resize_and_exit_are_delivered_to_subscribers() {
    let h = handle(&["sh"]);
    let (_snap, sub) = h.subscribe();
    h.resize(Size { cols: 90, rows: 20 }).unwrap();
    h.send_input(b"exit 7\n".to_vec()).unwrap();
    let (mut resized, mut exit) = (false, None);
    let end = Instant::now() + Duration::from_secs(10);
    while exit.is_none() && Instant::now() < end {
        match sub.recv_timeout(Duration::from_millis(100)) {
            Ok(Output::Resize(s)) => resized = s == Size { cols: 90, rows: 20 },
            Ok(Output::Exited(st)) => exit = Some(st),
            _ => {}
        }
    }
    assert!(resized, "resize not delivered");
    assert_eq!(exit.and_then(|e| e.code), Some(7));
}

#[test]
fn a_slow_subscriber_lags_without_slowing_the_terminal_and_can_resync() {
    let flood = handle(&["sh", "-c", "yes 'flood flood flood flood flood flood'"]);
    let (_s, slow) = flood.subscribe();
    // Never read: the queue fills and the subscriber is marked lagging while the terminal keeps running.
    let seq0 = flood.snapshot().seq;
    // Wait for progress rather than for a fixed time: a loaded CI runner may take longer than 700 ms to get through
    // 1000 chunks, which says nothing about the terminal being slowed by the subscriber.
    let end = Instant::now() + Duration::from_secs(20);
    while flood.snapshot().seq <= seq0 + 1000 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        flood.snapshot().seq > seq0 + 1000,
        "terminal kept processing output"
    );
    // By now the queue (never read) has certainly overflowed.

    assert_eq!(
        slow.recv_timeout(Duration::from_millis(100)),
        Ok(Output::Lagged)
    );
    let snap = flood.resync(&slow);
    assert!(snap.lines.iter().any(|l| l.starts_with("flood")));
    // After a resync, live data flows again.
    let next = slow.recv_timeout(Duration::from_secs(5));
    assert!(matches!(next, Ok(Output::Data(_))), "got {next:?}");
    flood.kill().unwrap();
}

#[test]
fn dropped_subscribers_are_forgotten() {
    let h = handle(&["sh"]);
    for _ in 0..50 {
        let (_s, sub) = h.subscribe();
        drop(sub);
    }
    h.send_input(b"echo still-fine\n".to_vec()).unwrap();
    wait_for(&h, "echo", |s| {
        s.lines.iter().any(|l| l.ends_with("still-fine"))
    });
    h.kill().unwrap();
}
