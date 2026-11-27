//! ADR-0088 Amendment A and ADR-0022: a command to a node (a keystroke is one) is not held up by a preview's bytes on the same link.
//! The hub writes preview frames only while no command waits, so a command waits at most for the frame already being written and for
//! what the socket already holds. Measured here through the hub's real node link (a fake node on the other end): the round trip of a
//! command alone, and while a preview floods the link toward the node. `cargo test -p agora-hub --test preview_priority -- --ignored
//! --nocapture` prints the numbers; the test fails when the flooded p95 is over 50 ms.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_hub::enroll::Lockout;
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_protocol::frame::{kinds, FIRST_DYNAMIC_STREAM, STREAM_COMMANDS, STREAM_CONTROL};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_request as cr, command_response};
use agora_protocol::ws::{self, Conn};
use agora_protocol::Frame;
use prost::Message;
use tokio::net::{TcpListener, TcpStream};

struct Tokens;
impl ClientAuth for Tokens {
    fn check(&self, _token: &str) -> bool {
        false
    }
}

fn f(stream: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    }
}

async fn recv(c: &mut Conn<TcpStream>) -> Frame {
    c.recv().await.unwrap().unwrap()
}

/// A node connected to the hub's real link loop; returns the hub and the node's end.
async fn linked() -> (Arc<ServerState>, Conn<TcpStream>) {
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec!["preview.v1".into()],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let hub = Arc::clone(&st);
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        let _ = server::serve_node(conn, "n", &hub.hub, &hub.support).await;
    });
    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect(s, &format!("ws://{addr}/")).await.unwrap();
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        capabilities: vec!["preview.v1".into()],
        build: "t".into(),
        locale: None,
    };
    c.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::WELCOME);
    c.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    assert_eq!(recv(&mut c).await.kind, kinds::RESUME_OK);
    (st, c)
}

fn ok(request_id: String, result: Option<v1::command_ok::Result>) -> Vec<u8> {
    v1::CommandResponse {
        request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk { result })),
    }
    .encode_to_vec()
}

fn percentile(v: &mut [Duration], p: f64) -> Duration {
    v.sort();
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement: run with --ignored --nocapture"]
async fn a_command_is_not_held_up_by_a_preview_flooding_the_link() {
    let (st, mut node) = linked().await;
    let preview_bytes = Arc::new(AtomicU64::new(0));
    // The node: answers every command at once (a `PreviewConnect` with a window large enough to flood), and counts preview bytes.
    let counted = Arc::clone(&preview_bytes);
    tokio::spawn(async move {
        loop {
            let fr = recv(&mut node).await;
            match (fr.stream_id, fr.kind) {
                (s, 0) if s == STREAM_COMMANDS || s >= FIRST_DYNAMIC_STREAM => {
                    let req = v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
                    let result = match req.command {
                        Some(cr::Command::PreviewConnect(_)) => Some(
                            v1::command_ok::Result::PreviewConnect(v1::PreviewConnectResult {
                                max_chunk_bytes: 16_384,
                                window_bytes: u32::MAX,
                                ..Default::default()
                            }),
                        ),
                        _ => None,
                    };
                    node.send(&f(STREAM_COMMANDS, 1, ok(req.request_id, result)))
                        .await
                        .unwrap();
                }
                (s, kinds::PREVIEW_DATA) if s >= FIRST_DYNAMIC_STREAM => {
                    counted.fetch_add(fr.payload.len() as u64, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    });
    let router = Arc::clone(st.hub.router());
    let round = || {
        let router = Arc::clone(&router);
        async move {
            let at = Instant::now();
            let reply = router
                .send(
                    "n",
                    v1::CommandRequest {
                        command: Some(cr::Command::AgentList(v1::AgentList::default())),
                        ..Default::default()
                    },
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(10), reply)
                .await
                .unwrap()
                .unwrap();
            at.elapsed()
        }
    };
    // Alone.
    let mut alone = Vec::new();
    for _ in 0..200 {
        alone.push(round().await);
    }
    // While a preview floods the link toward the node, 16 KiB frames as fast as the hub takes them.
    let link = router.connect_preview("n", "fwd-bench").await.unwrap();
    let flood = tokio::spawn(async move {
        let chunk = vec![0u8; 16_384];
        while link.send(kinds::PREVIEW_DATA, chunk.clone()).await {}
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = preview_bytes.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut flooded = Vec::new();
    for _ in 0..200 {
        flooded.push(round().await);
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let rate = (preview_bytes.load(Ordering::Relaxed) - before) as f64
        / started.elapsed().as_secs_f64()
        / 1e6;
    flood.abort();
    let (a50, a95) = (percentile(&mut alone, 0.5), percentile(&mut alone, 0.95));
    let (f50, f95) = (
        percentile(&mut flooded, 0.5),
        percentile(&mut flooded, 0.95),
    );
    println!("command round trip alone: p50 {a50:?} p95 {a95:?}; while a preview carries {rate:.0} MB/s to the node: p50 {f50:?} p95 {f95:?}");
    assert!(rate > 1.0, "the preview really flowed ({rate:.1} MB/s)");
    assert!(
        f95 < Duration::from_millis(50),
        "a command waited {f95:?} behind preview bytes"
    );
}
