//! #1150: an `agora` command whose reader went away (`agora … | head -1`, `2>&1 | head`) ends as it would have, never with a
//! panic. Each command runs with its stdout, its stderr, or both on a pipe whose reading end was closed before it started (so
//! every write fails, without a race against the reader), and must end with the exit code it has when both are read.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn home(name: &str) -> Home {
    let d = std::env::temp_dir().join(format!("agora-pipe-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Home(d)
}

fn agora(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(AGORA);
    c.args(args)
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .stdin(Stdio::null());
    c
}

/// A pipe nobody reads: its reading end is closed before the command starts.
fn closed() -> Stdio {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    writer.into()
}

#[derive(Clone, Copy, Debug)]
enum Cut {
    Stdout,
    Stderr,
    Both,
}

fn run_cut(home: &Path, args: &[&str], cut: Cut) -> Output {
    let mut c = agora(home, args);
    match cut {
        Cut::Stdout => c.stdout(closed()).stderr(Stdio::piped()),
        Cut::Stderr => c.stdout(Stdio::piped()).stderr(closed()),
        Cut::Both => c.stdout(closed()).stderr(closed()),
    };
    c.output().unwrap()
}

/// The command ends with the same code whichever of its outputs is cut short, and says nothing about a panic. What it printed
/// when read, for a test to see the command had something to print.
fn survives(home: &Path, args: &[&str]) -> Output {
    let read = agora(home, args).output().unwrap();
    let expected = read.status.code();
    assert_ne!(
        expected,
        Some(101),
        "{args:?} panics even when read: {read:?}"
    );
    for cut in [Cut::Stdout, Cut::Stderr, Cut::Both] {
        let o = run_cut(home, args, cut);
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(
            !stderr.contains("panicked"),
            "{args:?} with {cut:?} closed panicked: {stderr}"
        );
        assert_eq!(
            o.status.code(),
            expected,
            "{args:?} with {cut:?} closed ended differently: {o:?}"
        );
    }
    read
}

#[test]
fn the_usage_page_survives_a_closed_pipe() {
    let h = home("usage");
    survives(&h.0, &["--help"]);
    survives(&h.0, &["--version"]);
}

#[test]
fn an_error_survives_a_closed_pipe() {
    let h = home("error");
    let socket = h.0.join("missing.sock");
    let socket = socket.to_str().unwrap();
    // No node at that socket: an error line on stderr and exit 1.
    survives(&h.0, &["agent", "list", "--socket", socket]);
    survives(&h.0, &["agent", "kinds", "--socket", socket]);
    // Arguments not understood: two lines on stderr and exit 2.
    survives(&h.0, &["agent", "list", "--no-such-flag"]);
}

#[test]
fn hub_lists_survive_a_closed_pipe() {
    let h = home("hub");
    let hub = h.0.join("hub");
    let hub = hub.to_str().unwrap();
    survives(&h.0, &["hub", "user", "list", "--hub-dir", hub]);
    let added = agora(
        &h.0,
        &[
            "hub",
            "user",
            "add",
            "Ayse",
            "--role",
            "viewer",
            "--hub-dir",
            hub,
        ],
    )
    .output()
    .unwrap();
    assert!(added.status.success(), "{added:?}");
    survives(&h.0, &["hub", "user", "list", "--hub-dir", hub]);
    // A line per target, printed one by one.
    let added = agora(
        &h.0,
        &[
            "hub",
            "notify",
            "--webhook",
            "http://127.0.0.1:9/hook",
            "--hub-dir",
            hub,
        ],
    )
    .output()
    .unwrap();
    assert!(added.status.success(), "{added:?}");
    let listed = survives(&h.0, &["hub", "notify", "--hub-dir", hub]);
    assert!(
        listed.status.success() && !listed.stdout.is_empty(),
        "{listed:?}"
    );
    // A JSON line per audit row (adding Ayse made one).
    let rows = survives(&h.0, &["hub", "audit", "export", "--hub-dir", hub]);
    assert!(rows.status.success() && !rows.stdout.is_empty(), "{rows:?}");
}
