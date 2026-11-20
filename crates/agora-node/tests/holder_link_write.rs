//! A holder-link frame write keeps its time limit (`docs/specs/holder-link.md`: a write gives up after `FRAME_TIMEOUT`), also when a
//! backpressured peer takes part of a frame carrying a terminal only at the very end of that time (Codex review of #789).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use std::io::{Read, Write};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::holder_link::{write_frame, LinkError, FRAME_TIMEOUT, WRITE_SLICE};
use agora_platform::{HeldTerminal, ProcessLimits, PtyConfig};

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

#[test]
fn a_terminal_frame_drained_in_part_at_its_deadline_still_ends_by_it() {
    let dir = std::env::temp_dir().join(format!("agora-hlw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // An agent that only waits, so the terminal stays open while it is sent.
    let script = dir.join("wait.toml");
    std::fs::write(
        &script,
        "name = \"wait\"\n\n[[frame]]\ntext = \"waiting\"\ndelay_ms = 60000\n",
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
    // The peer's buffer is full before the frame begins.
    w.set_write_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let mut filler = 0usize;
    while let Ok(n) = w.write(&[0u8; 4096]) {
        filler += n;
    }
    w.set_write_timeout(Some(WRITE_SLICE)).unwrap();
    // Larger than the socket's buffer, so it can only go in part.
    let payload = vec![1u8; 2 << 20];
    let frame_len = 4 + payload.len();
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, Some(&handle));
        (res, began.elapsed(), w)
    });
    // Half a second before the deadline the peer takes the filler (and so lets part of the frame in), then reads no more until
    // the write has returned.
    std::thread::sleep(FRAME_TIMEOUT - Duration::from_millis(500));
    r.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0;
    while got < filler {
        let want = (filler - got).min(buf.len());
        match r.read(&mut buf[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    assert_eq!(got, filler);
    let (res, elapsed, _w) = writer.join().unwrap();
    // What of the frame went: its first bytes, with the terminal.
    let (mut part, mut terminal) = (0usize, false);
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
