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
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, Some(&handle));
        (res, began.elapsed(), w)
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
    let (res, elapsed, _w) = writer.join().unwrap();
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
            && elapsed <= FRAME_TIMEOUT + Duration::from_millis(300),
        "write_frame took {elapsed:?}, its limit is {FRAME_TIMEOUT:?}"
    );
}
