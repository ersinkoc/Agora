//! A local socket accepted from a non-blocking listener still blocks: its reads wait up to their timeout. On macOS an accepted
//! socket inherited the listener's O_NONBLOCK, so a read returned WouldBlock at once and the node's control and mesh loops spun a
//! core per idle connection.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::Read;
use std::time::{Duration, Instant};

#[test]
fn a_connection_from_a_non_blocking_listener_waits_for_its_read_timeout() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-accept-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).expect("listen");
    listener
        .set_nonblocking(true)
        .expect("non-blocking listener");
    let _client = agora_platform::local_connect(&path).expect("connect");
    let end = Instant::now() + Duration::from_secs(5);
    let mut conn = loop {
        match listener.accept() {
            Ok(c) => break c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    conn.set_read_timeout(Some(Duration::from_millis(100)))
        .expect("read timeout");
    let started = Instant::now();
    let mut buf = [0u8; 16];
    let read = conn.read(&mut buf);
    let waited = started.elapsed();
    assert!(read.is_err(), "nothing was written: {read:?}");
    assert!(
        waited >= Duration::from_millis(80),
        "the read returned after {waited:?}, not after its 100 ms timeout: the accepted socket does not block"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
