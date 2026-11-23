//! A holder-link frame write keeps its time limit (`docs/specs/holder-link.md`: a write gives up after `FRAME_TIMEOUT`), also when a
//! backpressured peer takes part of a frame carrying a terminal only at the very end of that time (Codex review of #789).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::holder_link::{write_frame, LinkError, FRAME_TIMEOUT, WRITE_SLICE};
use agora_platform::{HeldTerminal, ProcessLimits, PtyConfig};

/// The fake agent, built into this test's own target directory and profile (as `launch_options.rs` does).
fn fake_agent() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static BUILD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap());
        if p.parent().unwrap().ends_with("release") {
            command.arg("--release");
        }
        assert!(
            command.status().unwrap().success(),
            "building the fake agent failed"
        );
    });
    assert!(p.exists());
    p.to_string_lossy().into_owned()
}

#[test]
fn a_terminal_frame_drained_in_part_at_its_deadline_still_ends_by_it() {
    let dir = std::env::temp_dir().join(format!("agora-hlw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // An agent that only waits, so the terminal stays open while it is sent. It writes nothing: output nobody reads can keep a
    // killed child from finishing its exit on some systems (macOS), and the reap below would then wait for ever. `clear` is
    // off too: by default the fake agent writes a clear-screen sequence even with empty text.
    let script = dir.join("wait.toml");
    std::fs::write(
        &script,
        "name = \"wait\"\n\n[[frame]]\ntext = \"\"\nclear = false\ndelay_ms = 60000\n",
    )
    .unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let mut w = agora_platform::local_connect(&path).unwrap();
    let mut r = listener.accept().unwrap();
    let mut term = HeldTerminal::start(&PtyConfig {
        argv: vec![
            fake_agent(),
            "--script".into(),
            script.to_string_lossy().into_owned(),
        ],
        cwd: None,
        env: Vec::new(),
        cols: 80,
        rows: 24,
        limits: ProcessLimits::default(),
        sandbox: None,
        identity: None,
        env_remove: Vec::new(),
    })
    .unwrap();
    let handle = term.handle().unwrap();
    w.set_write_timeout(Some(WRITE_SLICE)).unwrap();
    // Larger than the socket's buffer, so it can only go in part. The peer reads nothing at first: the frame's first piece, with
    // the terminal, goes at once and the rest backs up. (The buffer is not filled beforehand: macOS refuses a send carrying a
    // descriptor into a full buffer outright, EMSGSIZE, where Linux waits; measured on a Mac, #789.)
    let payload = vec![1u8; 2 << 20];
    let frame_len = 4 + payload.len();
    let (done, result) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, Some(&handle));
        let _ = done.send((res, began.elapsed()));
        w
    });
    // Half a second before the deadline the peer takes a part of the frame (and so lets more of it in), then reads no more until
    // the write has returned.
    std::thread::sleep(FRAME_TIMEOUT - Duration::from_millis(500));
    r.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    const DRAIN: usize = 256 << 10;
    let mut buf = vec![0u8; 64 * 1024];
    let (mut part, mut terminal) = (0usize, false);
    while part < DRAIN {
        let want = (DRAIN - part).min(buf.len());
        match r.recv_with_terminal(&mut buf[..want]) {
            Ok((0, _)) | Err(_) => break,
            Ok((n, t)) => {
                part += n;
                terminal |= t.is_some();
            }
        }
    }
    // A write that never gives up fails the test instead of hanging it: closing the peer ends it.
    let Ok((res, elapsed)) = result.recv_timeout(Duration::from_secs(5)) else {
        drop(r);
        let _ = writer.join();
        panic!("write_frame did not return within {FRAME_TIMEOUT:?} and 4.5 s more");
    };
    let _w = writer.join().unwrap();
    // The rest of what went.
    while let Ok((n, t)) = r.recv_with_terminal(&mut buf) {
        if n == 0 {
            break;
        }
        part += n;
        terminal |= t.is_some();
    }
    drop(r);
    drop(listener);
    term.kill_and_reap();
    let _ = std::fs::remove_dir_all(&dir);
    println!("write_frame: {res:?} after {elapsed:?}; the peer got {part} of {frame_len} bytes, terminal={terminal}");
    assert!(terminal, "the terminal went with the first part");
    assert!(
        part > 0 && part < frame_len,
        "a part of the frame went: {part} of {frame_len}"
    );
    assert!(
        matches!(&res, Err(LinkError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
        "{res:?}"
    );
    assert!(
        elapsed >= FRAME_TIMEOUT - Duration::from_millis(50)
            && elapsed <= FRAME_TIMEOUT + Duration::from_millis(250),
        "write_frame took {elapsed:?}, its limit is {FRAME_TIMEOUT:?}"
    );
}

/// A frame to a peer that never reads, on a stream whose own write timeout is `own` (`None`: none at all). Returns what
/// `write_frame` answered and how long it took, or `None` when it had not returned 3 s after its deadline.
fn write_to_a_peer_that_never_reads(
    tag: &str,
    own: Option<Duration>,
) -> Option<(Result<(), LinkError>, Duration)> {
    let dir = std::env::temp_dir().join(format!("agora-hlw-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let mut w = agora_platform::local_connect(&path).unwrap();
    let r = listener.accept().unwrap();
    w.set_write_timeout(own).unwrap();
    // Far larger than the socket's buffer: most of it can never go.
    let payload = vec![1u8; 2 << 20];
    let (done, result) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, None);
        let _ = done.send((res, began.elapsed()));
    });
    let got = result
        .recv_timeout(FRAME_TIMEOUT + Duration::from_secs(3))
        .ok();
    // A write still blocked ends when the peer goes.
    drop(r);
    let _ = writer.join();
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);
    got
}

fn assert_ends_at_its_deadline(got: Option<(Result<(), LinkError>, Duration)>, own: &str) {
    let Some((res, elapsed)) = got else {
        panic!("write_frame with {own} did not return within {FRAME_TIMEOUT:?} and 3 s more");
    };
    println!("write_frame with {own}: {res:?} after {elapsed:?}");
    assert!(
        matches!(&res, Err(LinkError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
        "{res:?}"
    );
    // Not before its time, and not later than scheduling on a loaded machine explains: a send waits for room by `poll`, which ends
    // on time, and never for longer than the frame has left.
    assert!(
        elapsed >= FRAME_TIMEOUT - Duration::from_millis(50)
            && elapsed <= FRAME_TIMEOUT + Duration::from_millis(250),
        "write_frame with {own} took {elapsed:?}, its limit is {FRAME_TIMEOUT:?}"
    );
}

#[test]
fn a_frame_to_a_peer_that_never_reads_ends_at_its_deadline_with_no_write_timeout_set() {
    // Issue #830: the frame's deadline holds whatever the stream's own write timeout; with none, a blocked send waited for ever.
    let got = write_to_a_peer_that_never_reads("none", None);
    assert_ends_at_its_deadline(got, "no write timeout");
}

#[test]
fn a_frame_to_a_peer_that_never_reads_ends_at_its_deadline_with_a_long_write_timeout() {
    // A write timeout longer than a slice let one send run past the frame's deadline (the send began with time left).
    let got = write_to_a_peer_that_never_reads("long", Some(3 * WRITE_SLICE));
    assert_ends_at_its_deadline(got, "a 3 s write timeout");
}

#[test]
fn a_frame_to_a_peer_that_never_reads_ends_at_its_deadline_with_the_slice_set() {
    let got = write_to_a_peer_that_never_reads("slice", Some(WRITE_SLICE));
    assert_ends_at_its_deadline(got, "the write slice");
}
