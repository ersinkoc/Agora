//! A holder-link frame write keeps its time limit (`docs/specs/holder-link.md`: a write gives up after `FRAME_TIMEOUT`), also when a
//! backpressured peer takes part of a frame carrying a terminal only at the very end of that time (Codex review of #789).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use agora_node::holder_link::{write_frame, FRAME_TIMEOUT, WRITE_SLICE};
use agora_platform::{HeldTerminal, ProcessLimits, PtyConfig};

#[test]
fn a_terminal_frame_drained_in_part_at_its_deadline_still_ends_by_it() {
    let dir = std::env::temp_dir().join(format!("agora-hlw-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let mut w = agora_platform::local_connect(&path).unwrap();
    let mut r = listener.accept().unwrap();
    let mut term = HeldTerminal::start(&PtyConfig {
        argv: vec!["sleep".into(), "30".into()],
        cwd: None,
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
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
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, Some(&handle));
        (res.is_ok(), began.elapsed(), w)
    });
    // Half a second before the deadline the peer takes the filler (and so lets part of the frame in), then reads no more: the
    // send blocked at that moment returns that part only when its slice ends, at the deadline.
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
    let (ok, elapsed, _w) = writer.join().unwrap();
    drop(r);
    drop(listener);
    term.kill_and_reap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!ok, "the frame cannot have gone whole");
    assert!(
        elapsed <= FRAME_TIMEOUT + Duration::from_millis(300),
        "write_frame took {elapsed:?}, its limit is {FRAME_TIMEOUT:?}"
    );
}
