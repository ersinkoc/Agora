//! A holder-link frame write to a peer that never reads gives up at `FRAME_TIMEOUT` on every system (#1089: on Windows the pipe
//! write had no time limit, so `write_frame` waited for ever). No terminal goes with the frame, so the plain send path runs, which
//! Windows has too.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use agora_node::holder_link::{write_frame, LinkError, FRAME_TIMEOUT};

#[test]
fn a_frame_to_a_peer_that_never_reads_gives_up_at_its_deadline() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let dir = std::env::temp_dir().join(format!("agora-hlwb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let mut w = agora_platform::local_connect(&path).unwrap();
    let r = listener.accept().unwrap();
    // Far more than any system buffers between the two ends; the peer reads none of it.
    let payload = vec![3u8; 2 << 20];
    let (done, result) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let began = Instant::now();
        let res = write_frame(&mut w, &payload, None);
        let _ = done.send((res, began.elapsed()));
        w
    });
    // A write that never gives up fails the test instead of hanging it: closing the peer ends it.
    let Ok((res, elapsed)) = result.recv_timeout(FRAME_TIMEOUT + Duration::from_secs(5)) else {
        drop(r);
        let _ = writer.join();
        let _ = std::fs::remove_dir_all(&dir);
        panic!("write_frame did not give up within {FRAME_TIMEOUT:?} and 5 s more");
    };
    let _w = writer.join().unwrap();
    drop(r);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        matches!(&res, Err(LinkError::Io(e)) if e.kind() == std::io::ErrorKind::TimedOut),
        "{res:?}"
    );
    assert!(
        elapsed >= FRAME_TIMEOUT - Duration::from_millis(50),
        "gave up early: {elapsed:?}"
    );
    assert!(
        elapsed < FRAME_TIMEOUT + Duration::from_millis(500),
        "gave up late: {elapsed:?}"
    );
}
