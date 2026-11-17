//! Audit F1050: a failed accept does not close the tunnel. The tunnel's port must stay open while the shell (or `agora panel`)
//! holds the tunnel: a closed port turns every later page load away and leaves the fixed port free for another program (threat
//! model T27). The failure is real: this test process's open-file limit is lowered and every free handle is taken, so the tunnel's
//! accept fails with "too many open files" while a connection waits in its queue.
//!
//! Order is gated, not timed: the tunnel runs on a current-thread runtime that moves only inside `block_on`. The waiting connection
//! is put in the listen queue (and seen there in /proc/net/tcp) while the runtime is still; the runtime is then driven by
//! `yield_now`, which runs every other pending task (the tunnel's accept) before it returns; only then are the handles released.
//!
//! Its own test binary, since it changes a limit of the whole process. Needs Linux's /proc and `prlimit` (util-linux); elsewhere it
//! returns without checking.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::File;
use std::io::Write as _;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncReadExt;

fn limits() -> (u64, u64) {
    let text = std::fs::read_to_string("/proc/self/limits").unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("Max open files"))
        .unwrap();
    let mut n = line.split_whitespace().skip(3);
    (
        n.next().unwrap().parse().unwrap(),
        n.next().unwrap().parse().unwrap(),
    )
}

fn set_soft(soft: u64, hard: u64) -> bool {
    std::process::Command::new("prlimit")
        .args([
            "--pid",
            &std::process::id().to_string(),
            &format!("--nofile={soft}:{hard}"),
        ])
        .status()
        .is_ok_and(|s| s.success())
}

/// The listen-queue length of the socket listening on 127.0.0.1:`port`, or None when nothing listens there.
fn listen_queue(port: u16) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/net/tcp").unwrap();
    let want = format!("0100007F:{port:04X}");
    text.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        (f[1] == want && f[3] == "0A")
            .then(|| u64::from_str_radix(f[4].split(':').nth(1).unwrap(), 16).unwrap())
    })
}

/// A request addressed to another name: the tunnel answers it itself (421), so no hub is needed to see that it is alive.
fn misdirected(local: SocketAddr) -> String {
    format!(
        "GET / HTTP/1.1\r\nHost: rebind.example:{}\r\n\r\n",
        local.port()
    )
}

const REFUSED: &str =
    "HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

fn drive(rt: &tokio::runtime::Runtime) {
    rt.block_on(async {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    });
}

#[test]
fn a_failed_accept_keeps_the_port_and_the_tunnel_answers_again() {
    let proc_ok = ["/proc/self/limits", "/proc/self/fd", "/proc/net/tcp"]
        .iter()
        .all(|p| Path::new(p).exists());
    if !proc_ok {
        return;
    }
    let (soft, hard) = limits();
    if !set_soft(soft, hard) {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let ca = agora_hub::ca::Ca::generate("hub CA").unwrap();
    // The hub is never dialed here: every request is addressed to another name.
    let t = rt
        .block_on(agora_desktop_tunnel::open("localhost:9", ca.cert_pem(), 0))
        .unwrap();
    let local = t.local;
    drive(&rt);

    // Take every free handle but one, queue C1 with it, and see it in the listen queue.
    let open_now = std::fs::read_dir("/proc/self/fd").unwrap().count() as u64;
    assert!(set_soft(open_now + 64, hard));
    let mut held = Vec::new();
    while let Ok(f) = File::open("/dev/null") {
        held.push(f);
    }
    held.pop();
    let mut c1 = TcpStream::connect(local).unwrap();
    c1.write_all(misdirected(local).as_bytes()).unwrap();
    held.pop();
    let mut spins = 0u64;
    while listen_queue(local.port()) != Some(1) {
        spins += 1;
        assert!(spins < 1_000_000, "C1 never reached the listen queue");
        std::thread::yield_now();
    }
    held.push(File::open("/dev/null").unwrap());
    assert!(File::open("/dev/null").is_err(), "no handle is free");

    // The tunnel's accept runs now, with no handle free, and fails.
    drive(&rt);
    drop(held);
    assert!(set_soft(soft, hard));

    assert!(
        listen_queue(local.port()).is_some(),
        "the tunnel's port was closed by a failed accept"
    );
    assert!(
        std::net::TcpListener::bind(local).is_err(),
        "another program could take the tunnel's port"
    );
    let mut c2 = TcpStream::connect(local).unwrap();
    c2.write_all(misdirected(local).as_bytes()).unwrap();
    let read = |s: TcpStream| async move {
        s.set_nonblocking(true).unwrap();
        let mut s = tokio::net::TcpStream::from_std(s).unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_string(&mut out)).await;
        out
    };
    let (a, b) = rt.block_on(async { tokio::join!(read(c1), read(c2)) });
    assert_eq!(a, REFUSED, "the connection queued during the failure");
    assert_eq!(b, REFUSED, "a connection after the failure");
}
