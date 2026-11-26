//! Audit F2061: a failed `accept` is not the end of the hub's listener. The process running out of file descriptors for a moment
//! (EMFILE) used to end `server::run`'s accept loop for good: the listener was dropped, `agora hub` exited 0, and nothing restarts
//! an exit 0 (ADR-0062). Its own test binary, because it lowers this process's open-files limit. It needs `prlimit(1)` (util-linux)
//! to change that limit without a new dependency; where there is none (macOS, Windows) the test says so and checks nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use agora_hub::enroll::Lockout;
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_protocol::handshake::Support;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

struct NoTokens;
impl ClientAuth for NoTokens {
    fn check(&self, _: &str) -> bool {
        false
    }
}

fn state() -> Arc<ServerState> {
    Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(NoTokens),
        assets: None,
        sessions: None,
    })
}

const REQUEST: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";

/// Whether the hub answers the HTTP request already written on `s` within 10 s.
async fn answered(mut s: TcpStream) -> bool {
    let mut buf = [0u8; 16];
    matches!(
        tokio::time::timeout(Duration::from_secs(10), s.read(&mut buf)).await,
        Ok(Ok(n)) if n >= 8 && buf.starts_with(b"HTTP/1.")
    )
}

async fn asked(addr: std::net::SocketAddr) -> Option<TcpStream> {
    let mut s = TcpStream::connect(addr).await.ok()?;
    s.write_all(REQUEST).await.ok()?;
    Some(s)
}

/// This process's open-files limits `(soft, hard)`, or `None` without `prlimit(1)`.
fn nofile() -> Option<(u64, u64)> {
    let out = Command::new("prlimit")
        .args([
            "--pid",
            &std::process::id().to_string(),
            "--nofile",
            "--output=SOFT,HARD",
            "--noheadings",
            "--raw",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut it = text.split_whitespace().map(|v| v.parse::<u64>().ok());
    Some((it.next()??, it.next()??))
}

fn set_soft(soft: u64, hard: u64) {
    let ok = Command::new("prlimit")
        .args([
            "--pid",
            &std::process::id().to_string(),
            &format!("--nofile={soft}:{hard}"),
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "prlimit refused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_hub_keeps_accepting_after_running_out_of_file_descriptors_for_a_moment() {
    let Some((soft, hard)) = nofile() else {
        eprintln!("skipped: no prlimit(1) to lower this process's open-files limit");
        return;
    };
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let run = tokio::spawn(server::run(l, state(), None));
    assert!(answered(asked(addr).await.unwrap()).await);

    // No descriptor left but one, for the client's own socket: the hub's side of the connection finds none (EMFILE).
    set_soft(256, hard);
    let mut hogs = Vec::new();
    while let Ok(f) = std::fs::File::open("/dev/null") {
        hogs.push(f);
    }
    hogs.pop();
    let queued = asked(addr).await;
    // Lets the runtime see the queued connection while no descriptor is free (the connection is already in the kernel's queue).
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(hogs);
    set_soft(soft, hard);

    assert!(!run.is_finished(), "the accept loop ended");
    // The connection that came while descriptors ran out is served once they are back, and so is a new one.
    assert!(queued.is_some(), "the hub dropped its listener");
    assert!(answered(queued.unwrap()).await);
    assert!(answered(asked(addr).await.unwrap()).await);
}
