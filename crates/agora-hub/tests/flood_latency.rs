//! Architecture review R5: "measure input and approval latency during output floods before choosing scheduling machinery".
//!
//! One machine floods terminal output to a window while a window sends commands to another machine (the path keystrokes and
//! approvals take: client → hub → node → hub → client). The round trip is timed with and without the flood, with the flood going to
//! the measuring window itself and to another window. Loopback, so it times the hub's own share, which the budget caps at 5 ms
//! (`docs/design/performance.md`, keystroke → echo on a remote node).
//!
//! `cargo test --release -p agora-hub --test flood_latency -- --ignored --nocapture`
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agora_hub::enroll::Lockout;
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_protocol::frame::{
    kinds, FIRST_DYNAMIC_STREAM, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS,
};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request, command_response, event::Body};
use agora_protocol::ws::{self, Conn};
use agora_protocol::Frame;
use prost::Message;
use tokio::net::{TcpListener, TcpStream};

struct Tokens;
impl ClientAuth for Tokens {
    fn check(&self, token: &str) -> bool {
        token == "t"
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

/// The next frame, past the `Welcome` a window gets first (ADR-0106: a client's names a role, a node's does not); the tests of that
/// `Welcome` read it with `c.recv()`.
async fn recv(c: &mut Conn<TcpStream>) -> Frame {
    loop {
        let f = tokio::time::timeout(Duration::from_secs(10), c.recv())
            .await
            .expect("timeout")
            .unwrap()
            .expect("closed");
        let welcome = (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::WELCOME)
            && v1::Welcome::decode(f.payload.as_slice())
                .is_ok_and(|w| w.person_role() != v1::PersonRole::Unspecified);
        if !welcome {
            return f;
        }
    }
}

/// A connected machine `node`, announced online.
async fn node(st: &Arc<ServerState>, node: &'static str) -> Conn<TcpStream> {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let s2 = Arc::clone(st);
    tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let (conn, _) = ws::accept_with_token(s).await.unwrap();
        let _ = server::serve_node(conn, node, &s2.hub, &s2.support).await;
    });
    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect(s, &format!("ws://{addr}/")).await.unwrap();
    let hello = v1::Hello {
        role: v1::Role::Node as i32,
        protocol_generations: vec![1],
        ..Default::default()
    };
    c.send(&f(STREAM_CONTROL, kinds::HELLO, hello.encode_to_vec()))
        .await
        .unwrap();
    recv(&mut c).await;
    c.send(&f(
        STREAM_CONTROL,
        kinds::RESUME,
        v1::Resume::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    recv(&mut c).await;
    let online = v1::Event {
        producer_id: node.into(),
        seq: 1,
        at_ms: 1,
        body: Some(Body::NodeOnline(v1::NodeOnline {
            node_id: node.into(),
            ..Default::default()
        })),
    };
    c.send(&f(STREAM_EVENTS, kinds::EVENT, online.encode_to_vec()))
        .await
        .unwrap();
    loop {
        let fr = recv(&mut c).await;
        if (fr.stream_id, fr.kind) == (STREAM_EVENTS, kinds::EVENT_ACK) {
            return c;
        }
    }
}

async fn window(addr: std::net::SocketAddr) -> Conn<TcpStream> {
    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_token(s, &format!("ws://{addr}/"), "t")
        .await
        .unwrap();
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    while recv(&mut c).await.kind != kinds::SNAPSHOT {}
    c
}

fn ok(request_id: String, result: command_ok::Result) -> Vec<u8> {
    v1::CommandResponse {
        request_id,
        result: Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(result),
        })),
    }
    .encode_to_vec()
}

/// Machine `b`: answers every command at once (an `agent.list` with nothing in it).
fn answer_everything(mut b: Conn<TcpStream>) {
    tokio::spawn(async move {
        while let Ok(Some(fr)) = b.recv().await {
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 0) {
                let req = v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
                let body = ok(
                    req.request_id,
                    command_ok::Result::AgentList(v1::AgentListResult::default()),
                );
                if b.send(&f(STREAM_COMMANDS, 1, body)).await.is_err() {
                    return;
                }
            }
        }
    });
}

/// Machine `a`: takes the first terminal subscription, answers it, then writes 4 KiB output frames on its stream as fast as the hub
/// takes them until `stop`. Counts the bytes written.
fn flood(mut a: Conn<TcpStream>, stop: Arc<AtomicBool>, bytes: Arc<AtomicU64>) {
    tokio::spawn(async move {
        let stream = loop {
            let fr = a.recv().await.unwrap().unwrap();
            if fr.stream_id >= FIRST_DYNAMIC_STREAM && fr.kind == 0 {
                let req = v1::CommandRequest::decode(fr.payload.as_slice()).unwrap();
                let body = ok(req.request_id, command_ok::Result::Empty(v1::Empty {}));
                a.send(&f(STREAM_COMMANDS, 1, body)).await.unwrap();
                break fr.stream_id;
            }
        };
        let chunk = vec![b'x'; 4096];
        while !stop.load(Ordering::Relaxed) {
            if a.send(&f(stream, 0, chunk.clone())).await.is_err() {
                return;
            }
            bytes.fetch_add(4096, Ordering::Relaxed);
        }
    });
}

/// A window that watches machine `a`'s terminal and reads everything it is sent, so the flood keeps flowing.
async fn watch(c: &mut Conn<TcpStream>) {
    let sub = v1::CommandRequest {
        request_id: "sub".into(),
        target_node: "a".into(),
        command: Some(command_request::Command::TerminalSubscribe(
            v1::TerminalSubscribe {
                terminal_id: "t1".into(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    c.send(&f(FIRST_DYNAMIC_STREAM + 7, 0, sub.encode_to_vec()))
        .await
        .unwrap();
}

/// Sends `n` commands to machine `b`, one every 5 ms, reading everything else meanwhile; returns each round trip.
async fn round_trips(c: &mut Conn<TcpStream>, n: usize) -> Vec<Duration> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let id = format!("q{i}");
        let req = v1::CommandRequest {
            request_id: id.clone(),
            target_node: "b".into(),
            command: Some(command_request::Command::AgentList(v1::AgentList::default())),
            ..Default::default()
        };
        let sent = Instant::now();
        c.send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        loop {
            let fr = recv(c).await;
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1)
                && v1::CommandResponse::decode(fr.payload.as_slice())
                    .unwrap()
                    .request_id
                    == id
            {
                out.push(sent.elapsed());
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    out
}

/// Keeps reading a flooded window that does not measure.
fn drain(mut c: Conn<TcpStream>) {
    tokio::spawn(async move { while let Ok(Some(_)) = c.recv().await {} });
}

fn report(name: &str, mut v: Vec<Duration>, bytes: u64, secs: f64) {
    v.sort();
    let p = |q: f64| v[((v.len() as f64 - 1.0) * q) as usize].as_secs_f64() * 1000.0;
    println!(
        "{name}: n={} p50={:.2} ms p95={:.2} ms p99={:.2} ms max={:.2} ms; flood {:.1} MB/s",
        v.len(),
        p(0.5),
        p(0.95),
        p(0.99),
        p(1.0),
        bytes as f64 / 1e6 / secs
    );
}

#[derive(Clone, Copy, PartialEq)]
enum Flood {
    None,
    SameWindow,
    OtherWindow,
}

async fn scenario(name: &str, flood_to: Flood) {
    let v = measure(flood_to, 400).await;
    report(name, v.0, v.1, v.2);
}

/// The round trips of `n` commands to machine `b`, the flood's bytes meanwhile, and the seconds it took.
async fn measure(flood_to: Flood, n: usize) -> (Vec<Duration>, u64, f64) {
    let st = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
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
    tokio::spawn(server::run(l, Arc::clone(&st), None));
    let a = node(&st, "a").await;
    let b = node(&st, "b").await;
    answer_everything(b);
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let mut me = window(addr).await;
    match flood_to {
        Flood::None => drop(a),
        Flood::SameWindow => {
            flood(a, Arc::clone(&stop), Arc::clone(&bytes));
            watch(&mut me).await;
        }
        Flood::OtherWindow => {
            flood(a, Arc::clone(&stop), Arc::clone(&bytes));
            let mut other = window(addr).await;
            watch(&mut other).await;
            drain(other);
        }
    }
    // Let the flood reach full speed first.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = Instant::now();
    let b0 = bytes.load(Ordering::Relaxed);
    let v = round_trips(&mut me, n).await;
    let secs = started.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    (v, bytes.load(Ordering::Relaxed) - b0, secs)
}

/// R5: a node's answer reaches the window as soon as the hub has it. Every round trip used to wait for the window's loop to tick
/// (20 ms, `server.rs` `PUSH`): the median was never below 20 ms. The median of 20, so one stall of a loaded runner does not decide.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_commands_answer_does_not_wait_for_the_windows_next_tick() {
    let (mut v, _, _) = measure(Flood::None, 20).await;
    v.sort();
    assert!(v[v.len() / 2] < Duration::from_millis(10), "{v:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement"]
async fn commands_to_one_machine_while_another_floods_output() {
    scenario("no flood", Flood::None).await;
    scenario("flood to another window", Flood::OtherWindow).await;
    scenario("flood to this window", Flood::SameWindow).await;
}
