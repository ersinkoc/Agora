//! `LocalStream::wait_writable` and `set_read_timeout_or_closed` (issue #830).

#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(unix)]

use std::io::Write;
use std::time::{Duration, Instant};

use agora_platform::ReadTimeout;

fn pair(
    tag: &str,
) -> (
    agora_platform::LocalStream,
    agora_platform::LocalStream,
    std::path::PathBuf,
    agora_platform::LocalListener,
) {
    let dir = std::env::temp_dir().join(format!("agora-lsw-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let w = agora_platform::local_connect(&path).unwrap();
    let r = listener.accept().unwrap();
    (w, r, dir, listener)
}

#[test]
fn an_idle_stream_is_writable_at_once() {
    let (w, _r, dir, _l) = pair("idle");
    let began = Instant::now();
    assert!(w.wait_writable(Duration::from_secs(5)).unwrap());
    assert!(began.elapsed() < Duration::from_secs(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_full_stream_waits_the_whole_time_given_and_not_less() {
    let (mut w, _r, dir, _l) = pair("full");
    // The peer never reads: fill the send buffer until a write would block.
    let shortly = Duration::from_millis(50);
    w.set_write_timeout(Some(shortly)).unwrap();
    let chunk = vec![0u8; 64 << 10];
    while w.write(&chunk).is_ok() {}
    let want = Duration::from_millis(300);
    let began = Instant::now();
    let writable = w.wait_writable(want).unwrap();
    let took = began.elapsed();
    assert!(!writable, "a full stream is not writable");
    assert!(took >= want, "ended early: {took:?}");
    assert!(
        took < want + Duration::from_millis(500),
        "ended late: {took:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wait_with_no_time_limit_does_not_panic_and_ends_when_writable() {
    // Audit F78: `Duration::MAX` panicked on `Instant::now() + timeout` (unixhold.rs) before polling at all.
    let (mut w, mut r, dir, _l) = pair("max");
    let began = Instant::now();
    assert!(w.wait_writable(Duration::MAX).unwrap());
    assert!(began.elapsed() < Duration::from_secs(1));
    // Full, then the peer drains: the unlimited wait ends once there is room, writable.
    w.set_write_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let chunk = vec![0u8; 64 << 10];
    while w.write(&chunk).is_ok() {}
    let drain = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = vec![0u8; 64 << 10];
        while matches!(r.read(&mut buf), Ok(n) if n > 0) {}
    });
    assert!(w.wait_writable(Duration::MAX).unwrap());
    drop(w);
    drain.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stream_whose_peer_closed_is_ready_so_the_send_reports_it() {
    let (w, r, dir, _l) = pair("closed");
    drop(r);
    assert!(w.wait_writable(Duration::from_secs(5)).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_timeout_is_set_or_reported_as_the_peer_having_closed() {
    let (w, r, dir, _l) = pair("rt");
    assert_eq!(
        w.set_read_timeout_or_closed(Some(Duration::from_secs(1)))
            .unwrap(),
        ReadTimeout::Set
    );
    drop(r);
    // Linux sets it still, macOS answers EINVAL: both are the two Ok answers; neither is an error.
    let after = w
        .set_read_timeout_or_closed(Some(Duration::from_secs(2)))
        .unwrap();
    assert!(matches!(after, ReadTimeout::Set | ReadTimeout::PeerClosed));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_read_timeout_after_the_peer_closed_with_bytes_unread_is_not_an_error() {
    // macOS refuses the timeout (EINVAL) here too; the stream is hung up while its last bytes wait, so it is `PeerClosed` and the
    // bytes are still read. Linux sets it.
    let (w, mut r, dir, _l) = pair("unread");
    {
        let mut w = w;
        w.write_all(b"last").unwrap();
    }
    let after = r
        .set_read_timeout_or_closed(Some(Duration::from_secs(2)))
        .unwrap();
    assert!(matches!(after, ReadTimeout::Set | ReadTimeout::PeerClosed));
    let mut got = Vec::new();
    std::io::Read::read_to_end(&mut r, &mut got).unwrap();
    assert_eq!(got, b"last");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zero_timeout_is_an_error_and_not_a_closed_peer() {
    let (w, _r, dir, _l) = pair("zero");
    let e = w
        .set_read_timeout_or_closed(Some(Duration::ZERO))
        .unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    let _ = std::fs::remove_dir_all(&dir);
}
