//! PTY-backed tests, on every platform with a pseudo-terminal (ADR-0045).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_node::terminal::{TerminalHandle, TerminalOptions};
use agora_platform::{spawn, PtyConfig};
use agora_vt::Size;

/// Whether `sh` (the test agent) can be found on `PATH`, as in `crates/agora/tests/cli.rs`. Every Unix has it; on Windows it comes
/// with Git's `usr\bin`, which CI runners put on `PATH` and a default Git for Windows install does not (#972).
fn sh_on_path() -> bool {
    let found = std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| d.join("sh").is_file() || d.join("sh.exe").is_file())
    });
    if !found {
        eprintln!("skipped: no `sh` on PATH (on Windows, add Git's usr\\bin to PATH)");
    }
    found
}

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
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        // `cargo test` only builds binaries of the package under test; build the fake agent on demand.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = std::process::Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success();
        assert!(ok, "building agora-fake-agent failed");
    });
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
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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

/// A request for fewer columns than the screen keeps (F701: the screen is never narrower than two) is the size the screen has, so the
/// process's own terminal and every subscriber agree with it, and asking again for the same narrow size reaches no one.
#[test]
fn a_request_below_the_minimum_width_is_the_size_everyone_has() {
    if !sh_on_path() {
        return;
    }
    let h = handle(&["sh"]);
    let (_snap, sub) = h.subscribe();
    for _ in 0..3 {
        h.resize(Size { cols: 1, rows: 7 }).unwrap();
    }
    let real = Size { cols: 2, rows: 7 };
    assert_eq!(h.snapshot().size, real);
    // `stty size` prints rows then columns of the process's terminal.
    let out = std::env::temp_dir().join(format!("agora-narrow-{}", std::process::id()));
    let _ = std::fs::remove_file(&out);
    h.send_input(format!("stty size > '{}'\n", out.display()).into_bytes())
        .unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let mut said = String::new();
    while Instant::now() < end && said.trim().is_empty() {
        std::thread::sleep(Duration::from_millis(100));
        said = std::fs::read_to_string(&out).unwrap_or_default();
    }
    let _ = std::fs::remove_file(&out);
    assert_eq!(said.trim(), "7 2", "the process sees the screen's size");
    // The shell's echo of the `stty` line and its prompt arrive on the same subscription: only the resizes count, and a byte frame
    // in front of them must not end the count (it did, once, on a slow runner).
    let mut resizes = Vec::new();
    while let Ok(out) = sub.recv_timeout(Duration::from_millis(100)) {
        if let Output::Resize(s) = out {
            resizes.push(s);
        }
    }
    assert_eq!(resizes, [real], "one resize, to the real size");
    h.kill().unwrap();
}

#[test]
fn a_resize_to_the_same_size_reaches_no_one() {
    if !sh_on_path() {
        return;
    }
    let h = handle(&["sh"]);
    let (snap, sub) = h.subscribe();
    let same = snap.size;
    let seq = h.snapshot().seq;
    h.resize(same).unwrap();
    h.resize(Size { cols: 91, rows: 21 }).unwrap();
    h.resize(Size { cols: 91, rows: 21 }).unwrap();
    let mut resizes = Vec::new();
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        if let Ok(Output::Resize(s)) = sub.recv_timeout(Duration::from_millis(100)) {
            resizes.push(s);
        }
    }
    assert_eq!(
        resizes,
        [Size { cols: 91, rows: 21 }],
        "only the change is delivered"
    );
    assert!(h.snapshot().seq > seq);
    h.kill().unwrap();
}

#[test]
fn a_slow_subscriber_lags_without_slowing_the_terminal_and_can_resync() {
    if !sh_on_path() {
        return;
    }
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
    if !sh_on_path() {
        return;
    }
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

#[test]
fn a_subscriber_gets_the_screen_with_its_colours_and_the_same_cells() {
    if !sh_on_path() {
        return;
    }
    let h = handle(&["sh"]);
    h.send_input(b"printf '\\033[1;31mRED\\033[0m \\033[38;5;208mORANGE\\033[0m \\033[48;2;1;2;3mRGB\\033[0m\\n'\n".to_vec())
        .unwrap();
    wait_for(&h, "coloured line", |s| {
        s.lines.iter().any(|l| l.contains("RED ORANGE RGB"))
    });
    let (snap, _sub) = h.subscribe();
    let ansi = snap.to_ansi();
    let text = String::from_utf8_lossy(&ansi);
    assert!(text.contains("\x1b[0;1;31mRED"), "{text:?}");
    assert!(text.contains("\x1b[0;38;5;208mORANGE"), "{text:?}");
    assert!(text.contains("\x1b[0;48;2;1;2;3mRGB"), "{text:?}");
    // Painted on a fresh terminal of the same size, the repaint gives back the node's own screen and cursor.
    let mut mirror = AlacrittyTerminal::new(snap.size);
    mirror.feed(&ansi);
    assert_eq!(lines(&mirror), snap.lines);
    assert_eq!(
        (mirror.cursor().row, mirror.cursor().col),
        (snap.cursor.row, snap.cursor.col)
    );
    // A plain snapshot (what detection reads) does not pay for the repaint.
    assert!(h.snapshot().repaint.is_none());
    h.kill().unwrap();
}

/// A client that joins after the program set its colour and input modes draws what follows like the program's own screen, and its
/// keys and pastes are encoded the way the program asked for (audit F040: the repaint used to leave both at their defaults).
#[test]
fn a_subscriber_joining_mid_stream_gets_the_programs_style_and_input_modes() {
    if !sh_on_path() {
        return;
    }
    let h = handle(&[
        "sh",
        "-c",
        "printf '\\033[?2004h\\033[?1h\\033[1;31mred-'; read x; printf 'more'; read y",
    ]);
    wait_for(&h, "first output", |s| {
        s.lines.iter().any(|l| l.contains("red-"))
    });
    let (snap, sub) = h.subscribe();
    // The client had other modes and colours from earlier output; the repaint replaces them.
    let mut mirror = AlacrittyTerminal::new(snap.size);
    mirror.feed(b"\x1b[?2004l\x1b[?1l\x1b[32m");
    mirror.feed(&snap.to_ansi());
    // The program goes on only once the client has its repaint.
    h.send_input(b"go\n".to_vec()).unwrap();
    drain(&sub, &mut mirror, |t| {
        lines(t).iter().any(|l| l.contains("more"))
    });
    wait_for(&h, "more", |s| s.lines.iter().any(|l| l.contains("more")));
    let (row, col) = (0..mirror.size().rows)
        .find_map(|r| mirror.line(r).find("more").map(|c| (r, c as u16)))
        .unwrap();
    let theirs = h.inspect(|t| t.cell(row, col).map(|c| c.style)).unwrap();
    assert!(theirs.bold && theirs.fg == agora_vt::Color::Indexed(1));
    assert_eq!(mirror.cell(row, col).map(|c| c.style), Some(theirs));
    assert_eq!(mirror.modes(), h.snapshot().modes);
    assert!(mirror.modes().bracketed_paste && mirror.modes().application_cursor);
    h.kill().unwrap();
}

/// Removes a test's folder when the test ends, also when it panics: the descendant stops once its `STARTED` file is gone.
#[cfg(unix)]
struct RemoveOnDrop(std::path::PathBuf);

#[cfg(unix)]
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::write(self.0.join("STOP"), b"");
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// F1480: a program that left a descendant outside the terminal's process group (its own session) writing to the terminal every
/// 0.1 s, in `dir`; the descendant ends when `STOP` appears there or `dir` is removed (at most 30 s). `None` without `sh` and `setsid`.
#[cfg(unix)]
fn with_busy_descendant(dir: &std::path::Path, tail: &str) -> Option<TerminalHandle> {
    let setsid = std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("setsid").is_file()));
    if !sh_on_path() || !setsid {
        eprintln!("skipped: needs sh and setsid on PATH");
        return None;
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    // The program goes on only once the descendant runs on its own.
    let script = format!(
        "setsid sh -c ': > STARTED; i=0; while [ -e STARTED ] && [ ! -e STOP ] && [ $i -lt 300 ]; do echo tick; i=$((i+1)); sleep 0.1; done' & \
         while [ ! -e STARTED ]; do sleep 0.05; done; echo LEADER-DONE; {tail}"
    );
    let mut cfg = PtyConfig::new(vec!["sh".into(), "-c".into(), script]);
    cfg.cwd = Some(dir.to_path_buf());
    let h = TerminalHandle::start(spawn(&cfg).expect("spawn"), TerminalOptions::default())
        .expect("start");
    wait_for(&h, "the descendant writing after the program's line", |s| {
        s.lines
            .iter()
            .position(|l| l.contains("LEADER-DONE"))
            .is_some_and(|at| s.lines.iter().skip(at + 1).any(|l| l.contains("tick")))
    });
    Some(h)
}

/// F1480: the end of a program is seen while such a descendant keeps writing (the actor never goes idle), so the supervisor reports
/// it and restarts a crash.
#[cfg(unix)]
#[test]
fn the_end_is_seen_while_a_descendant_keeps_writing() {
    let dir = std::env::temp_dir().join(format!("agora-f1480-end-{}", std::process::id()));
    let _cleanup = RemoveOnDrop(dir.clone());
    let Some(h) = with_busy_descendant(&dir, "exit 3") else {
        return;
    };
    let end = Instant::now() + Duration::from_secs(5);
    while h.exit_status().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    let seen = h.exit_status();
    std::fs::write(dir.join("STOP"), b"").unwrap();
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(seen.and_then(|s| s.code), Some(3), "the end was not seen");
}

/// F1480: dropping the handle (a stop, which the supervisor does under its lock) returns while such a descendant keeps writing.
#[cfg(unix)]
#[test]
fn a_stop_returns_while_a_descendant_keeps_writing() {
    let dir = std::env::temp_dir().join(format!("agora-f1480-stop-{}", std::process::id()));
    let _cleanup = RemoveOnDrop(dir.clone());
    let Some(h) = with_busy_descendant(&dir, "sleep 30") else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let dropper = std::thread::spawn(move || {
        drop(h);
        let _ = tx.send(());
    });
    let returned = rx.recv_timeout(Duration::from_secs(5)).is_ok();
    // The descendant ends, so the drop ends either way.
    std::fs::write(dir.join("STOP"), b"").unwrap();
    dropper.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(returned, "dropping the handle did not return");
}

/// F1481: a subscription made after the process ended (the node still holds its terminal) ends with that exit, so a window that
/// subscribed late gets the program's exit code rather than an unknown one when the terminal is let go.
#[test]
fn a_subscription_after_the_end_gets_the_exit() {
    if !sh_on_path() {
        return;
    }
    let h = handle(&["sh", "-c", "exit 7"]);
    wait_for(&h, "exit", |s| s.exit.is_some());
    let (snap, sub) = h.subscribe();
    assert_eq!(snap.exit.and_then(|e| e.code), Some(7));
    match sub.recv_timeout(Duration::from_secs(2)) {
        Ok(Output::Exited(st)) => assert_eq!(st.code, Some(7)),
        other => panic!("no exit for a late subscriber: {other:?}"),
    }
}
