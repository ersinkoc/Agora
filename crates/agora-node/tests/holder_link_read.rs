//! A holder-link frame read keeps its time limit (`docs/specs/holder-link.md`: a frame begun must end within `FRAME_TIMEOUT`), also
//! for the receive that would complete the frame: a last chunk that arrives after the deadline fails the read (Codex review of #789).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use std::io::Write;
use std::time::{Duration, Instant};

use agora_node::holder_link::{read_frame, LinkError, FRAME_TIMEOUT};

/// The peer sends all of a frame but its last 10 bytes at once and those `late` after; returns what `read_frame` gave and when.
fn last_chunk_after(tag: &str, late: Duration) -> (Result<bool, LinkError>, Duration) {
    let dir = std::env::temp_dir().join(format!("agora-hlr-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let mut w = agora_platform::local_connect(&path).unwrap();
    let mut r = listener.accept().unwrap();
    // A wait for a frame to begin longer than the frame's whole time (the node's client waits 10 s for a reply).
    let wait = Duration::from_secs(10);
    let payload = vec![7u8; 100];
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&payload);
    let split = frame.len() - 10;
    let (head, tail) = (frame[..split].to_vec(), frame[split..].to_vec());
    // The frame's first bytes are queued before the clock starts (the read's own clock starts at them), so a slow start of the
    // peer's thread cannot count against it.
    w.write_all(&head).unwrap();
    let peer = std::thread::spawn(move || {
        std::thread::sleep(late);
        let _ = w.write_all(&tail);
        w
    });
    let began = Instant::now();
    let got = read_frame(&mut r, wait).map(|f| f.is_some_and(|(p, _)| p == payload));
    let elapsed = began.elapsed();
    let _w = peer.join().unwrap();
    drop(listener);
    let _ = std::fs::remove_dir_all(&dir);
    println!("{tag}: last chunk after {late:?} → {got:?} after {elapsed:?}");
    (got, elapsed)
}

#[test]
fn a_last_chunk_after_the_deadline_fails_the_read_at_the_deadline() {
    let (got, elapsed) = last_chunk_after("late", FRAME_TIMEOUT + Duration::from_millis(300));
    assert!(
        matches!(&got, Err(LinkError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
        "{got:?}"
    );
    assert!(
        elapsed <= FRAME_TIMEOUT + Duration::from_millis(200),
        "read_frame took {elapsed:?}, its limit is {FRAME_TIMEOUT:?}"
    );
}

#[test]
fn a_last_chunk_in_time_completes_the_frame() {
    let (got, _) = last_chunk_after("in-time", FRAME_TIMEOUT - Duration::from_millis(500));
    assert!(matches!(got, Ok(true)), "{got:?}");
}
