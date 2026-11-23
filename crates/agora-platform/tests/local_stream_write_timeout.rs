//! `LocalStream::set_write_timeout` bounds a send on every system, and a send that timed out sent nothing, so trying it again keeps
//! the stream whole (#1089: on Windows the pipe write had no time limit, and a cancelled pipe write larger than the pipe's buffer can
//! report 0 bytes although its reader already took some).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

fn pair(
    tag: &str,
) -> (
    agora_platform::LocalStream,
    agora_platform::LocalStream,
    std::path::PathBuf,
    agora_platform::LocalListener,
) {
    let dir = std::env::temp_dir().join(format!("agora-lswt-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = agora_platform::local_listen(&path).unwrap();
    let w = agora_platform::local_connect(&path).unwrap();
    let r = listener.accept().unwrap();
    (w, r, dir, listener)
}

fn timed_out(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

#[test]
fn a_send_to_a_peer_that_never_reads_ends_at_its_write_timeout() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (mut w, _r, dir, _l) = pair("never");
    let limit = Duration::from_millis(200);
    w.set_write_timeout(Some(limit)).unwrap();
    let chunk = vec![7u8; 64 << 10];
    // Fill whatever the system buffers; the send that cannot go must end at the timeout, not wait for ever.
    let mut sent = 0usize;
    let mut last;
    loop {
        let began = Instant::now();
        let r = w.write(&chunk);
        last = began.elapsed();
        match r {
            Ok(n) => {
                sent += n;
                assert!(sent < 64 << 20, "64 MiB went to a peer that never reads");
            }
            Err(e) if timed_out(&e) => break,
            Err(e) => panic!("send failed: {e}"),
        }
    }
    assert!(
        last >= limit - Duration::from_millis(20),
        "ended early: {last:?}"
    );
    assert!(
        last < limit + Duration::from_secs(1),
        "ended late: {last:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_zero_write_timeout_is_refused() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (w, _r, dir, _l) = pair("zero");
    let e = w.set_write_timeout(Some(Duration::ZERO)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A writer that keeps timing out against a slow reader and tries each timed-out send again, as `holder_link::write_frame` does,
/// delivers every byte once and in order. Each send offers 256 KiB, more than a Windows pipe holds, so a cancelled pipe write that
/// had been partly taken would make the retry send those bytes twice (seen with the time limit and no size cap).
#[test]
fn sends_that_time_out_and_are_tried_again_deliver_every_byte_once() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let (mut w, mut r, dir, _l) = pair("retry");
    let data: Vec<u8> = (0..(1usize << 20))
        .map(|i| (i % 251) as u8 ^ (i >> 16) as u8)
        .collect();
    let total = data.len();
    let sending = data.clone();
    w.set_write_timeout(Some(Duration::from_millis(5))).unwrap();
    // The writer stops when it has sent everything, when the reader is gone (a send then fails), or after 30 s: a writer that lost
    // count would otherwise send its duplicates for ever.
    let writer = std::thread::spawn(move || {
        let data = sending;
        let began = Instant::now();
        let mut sent = 0usize;
        let mut timeouts = 0usize;
        while sent < data.len() && began.elapsed() < Duration::from_secs(30) {
            let end = data.len().min(sent + (256 << 10));
            match w.write(&data[sent..end]) {
                Ok(0) => break,
                Ok(n) => sent += n,
                Err(e) if timed_out(&e) => timeouts += 1,
                Err(_) => break,
            }
        }
        (sent, timeouts, w)
    });
    r.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut got = Vec::with_capacity(total);
    let mut buf = vec![0u8; 3000];
    let began = Instant::now();
    while got.len() < total {
        // A reader slower than the writer: a few KiB, then a pause, so sends wait and time out while part is being taken.
        let want = buf.len().min(total - got.len());
        let n = r.read(&mut buf[..want]).unwrap();
        assert!(n > 0, "the stream ended after {} bytes", got.len());
        got.extend_from_slice(&buf[..n]);
        if got.len() % 4 == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            began.elapsed() < Duration::from_secs(60),
            "the reader took too long"
        );
    }
    // Checked before the writer is waited for: a writer that lost count is still sending.
    let first_bad = got.iter().zip(&data).position(|(a, b)| a != b);
    // Nothing more may come: a duplicated byte would be left over.
    r.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let extra = r.read(&mut buf);
    // Closing the reader ends a writer still sending (its next send fails).
    drop(r);
    let (sent, timeouts, _w) = writer.join().unwrap();
    assert_eq!(
        first_bad, None,
        "the stream differs from what was sent (the writer counted {sent} of {total} bytes sent)"
    );
    assert!(
        matches!(&extra, Err(e) if timed_out(e)),
        "bytes after the end: {extra:?}"
    );
    assert_eq!(sent, total, "the writer did not finish");
    assert!(
        timeouts > 0,
        "no send timed out, so the retry was not exercised"
    );
    println!("{timeouts} sends timed out and were tried again");
    let _ = std::fs::remove_dir_all(&dir);
}
