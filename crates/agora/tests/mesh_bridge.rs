//! ADR-0075 decision 6: an `agora mesh stdio` bridge outlives its connection. These tests run the real bridge process against a real
//! local socket served by this file (a small MCP server that, like the node's, answers nothing before `initialize` on a connection),
//! and stop and start that server at the same path.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agora_platform::LocalStream;
use serde_json::{json, Value};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

/// What a server instance saw.
#[derive(Default)]
struct Seen {
    connections: AtomicUsize,
    initializes: AtomicUsize,
    initialized_notes: AtomicUsize,
    hung: AtomicUsize,
}

/// How a server instance answers `initialize`.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// As the node does.
    Accept,
    /// A peer that refuses it.
    Refuse,
    /// A peer that takes the connection and never answers it.
    Withhold,
    /// A node that sends a line of its own right behind the answer.
    Chatty,
}

/// A node's agent endpoint for the test.
struct FakeNode {
    stop: Arc<AtomicBool>,
    streams: Arc<Mutex<Vec<LocalStream>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    seen: Arc<Seen>,
}

impl FakeNode {
    fn start(path: &Path, mode: Mode) -> Self {
        // #1150: a bridge another test thread is starting holds a copy of every open descriptor from its fork until it runs its
        // program, the stopped node's `<path>.lock` too, and the system counts that copy as holding the lock: a node started again at
        // once can be refused for that moment. The node's mesh server waits it out the same way (`RELISTEN_WAIT`, #1093); a lock
        // still held after the wait is a real holder and fails the test.
        let until = Instant::now() + Duration::from_secs(5);
        let listener = loop {
            match agora_platform::local_listen(path) {
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                other => break other.unwrap(),
            }
        };
        listener.set_nonblocking(true).unwrap();
        let (stop, streams, seen) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(Vec::<LocalStream>::new())),
            Arc::new(Seen::default()),
        );
        let thread = {
            let (stop, streams, seen) = (stop.clone(), streams.clone(), seen.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok(stream) => {
                            let serial = seen.connections.fetch_add(1, Ordering::SeqCst) + 1;
                            streams.lock().unwrap().push(stream.try_clone().unwrap());
                            let seen = seen.clone();
                            std::thread::spawn(move || serve(stream, serial, mode, &seen));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
                // The listener drops here: the socket path is removed, as when the node goes.
            })
        };
        Self {
            stop,
            streams,
            thread: Some(thread),
            seen,
        }
    }

    /// The node goes away: its listener closes and every connection ends.
    fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for s in self.streams.lock().unwrap().drain(..) {
            let _ = s.shutdown();
        }
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}

impl Drop for FakeNode {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(stream: LocalStream, serial: usize, mode: Mode, seen: &Seen) {
    let mut out = stream.try_clone().unwrap();
    let mut initialized = false;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        let msg: Value = serde_json::from_str(&line).unwrap();
        let (id, method) = (msg.get("id").cloned(), msg["method"].as_str().unwrap_or(""));
        let reply = match (method, id) {
            ("initialize", Some(id)) => {
                seen.initializes.fetch_add(1, Ordering::SeqCst);
                if mode == Mode::Withhold {
                    continue;
                }
                if mode != Mode::Refuse {
                    initialized = true;
                    let answer = json!({"jsonrpc":"2.0","id":id,"result":{
                        "protocolVersion": msg["params"]["protocolVersion"],
                        "serverInfo":{"name":"fake"}}});
                    if mode == Mode::Chatty {
                        // One write: the answer and a line the node sends on its own.
                        let note = json!({"jsonrpc":"2.0","method":"notifications/chatty"});
                        if write!(out, "{answer}\n{note}\n").is_err() {
                            return;
                        }
                        continue;
                    }
                    answer
                } else {
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"no"}})
                }
            }
            ("notifications/initialized", None) => {
                seen.initialized_notes.fetch_add(1, Ordering::SeqCst);
                continue;
            }
            ("tools/call", Some(id)) if initialized => {
                if msg["params"]["name"] == "hang" {
                    seen.hung.fetch_add(1, Ordering::SeqCst);
                    continue;
                }
                json!({"jsonrpc":"2.0","id":id,"result":{"connection":serial}})
            }
            (_, Some(id)) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"uninitialized"}})
            }
            _ => continue,
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

/// The agent's MCP client: owns the bridge process and its pipes.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Value>,
    next: u64,
}

impl Client {
    fn start(socket: &Path) -> Self {
        Self::spawn(socket, true)
    }

    /// `listening: false` closes the client's end of the bridge's stdout at once.
    fn spawn(socket: &Path, listening: bool) -> Self {
        let mut child = Command::new(AGORA)
            .args(["mesh", "stdio", "--socket"])
            .arg(socket)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        if !listening {
            drop(stdout);
            return Self {
                child,
                stdin,
                lines,
                next: 0,
            };
        }
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tx.send(serde_json::from_str(&line).unwrap()).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            next: 0,
        }
    }

    fn send(&mut self, msg: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    /// Sends a request; returns its id.
    fn request(&mut self, method: &str, params: Value) -> u64 {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        id
    }

    fn answer(&self, wait: Duration) -> Option<Value> {
        self.lines.recv_timeout(wait).ok()
    }

    fn call(&mut self, tool: &str, wait: Duration) -> Value {
        let id = self.request("tools/call", json!({"name": tool, "arguments": {}}));
        let a = self.answer(wait).expect("an answer");
        assert_eq!(a["id"], id, "{a}");
        a
    }

    fn initialize(&mut self) {
        let id = self.request("initialize", json!({"protocolVersion":"2025-03-26"}));
        let a = self
            .answer(Duration::from_secs(5))
            .expect("initialize answer");
        assert_eq!(a["id"], id);
        assert!(a["result"].is_object(), "{a}");
        self.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }

    fn alive(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    fn close_and_wait(&mut self, wait: Duration) -> Option<i32> {
        self.stdin = None;
        let t = Instant::now();
        while t.elapsed() < wait {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s.code();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        None
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agora-mb-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("a.sock")
}

fn unavailable(a: &Value) -> bool {
    a["error"]["data"]["code"] == "mesh.unavailable"
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_bridge_reconnects_to_the_same_path_and_keeps_its_clients_session() {
    let path = socket("reconnect");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    let a = client.call("echo", Duration::from_secs(5));
    assert_eq!(a["result"]["connection"], 1, "{a}");

    // A call is in flight when the node goes.
    let hung = client.request("tools/call", json!({"name": "hang", "arguments": {}}));
    wait_for("the node holds the call", || {
        node.seen.hung.load(Ordering::SeqCst) == 1
    });
    node.stop();
    let a = client.answer(Duration::from_secs(5)).expect("an answer");
    assert_eq!(a["id"], hung);
    assert!(unavailable(&a), "{a}");
    assert_eq!(a["error"]["code"], -32000);
    assert_eq!(a["error"]["data"]["params"], json!({}));
    assert!(client.alive(), "the bridge must stay up for its client");

    // While there is no node, a call is answered at once with the same error, not left hanging.
    let t = Instant::now();
    let a = client.call("echo", Duration::from_secs(2));
    assert!(unavailable(&a), "{a}");
    assert!(t.elapsed() < Duration::from_secs(1));

    // The node comes back at the same path, with no memory of the old connection.
    let node = FakeNode::start(&path, Mode::Accept);
    let t = Instant::now();
    let answer = loop {
        let a = client.call("echo", Duration::from_secs(5));
        if !unavailable(&a) {
            break a;
        }
        assert!(t.elapsed() < Duration::from_secs(8), "never reconnected");
        std::thread::sleep(Duration::from_millis(50));
    };
    // Served by the new instance (its first connection), which was initialized by the replayed handshake, not by cached state.
    assert_eq!(answer["result"]["connection"], 1, "{answer}");
    assert_eq!(node.seen.initializes.load(Ordering::SeqCst), 1);
    wait_for("the replayed initialized notification", || {
        node.seen.initialized_notes.load(Ordering::SeqCst) == 1
    });
    // The client's own initialize answer was not repeated to it: the next line it reads answers the next request.
    let a = client.call("echo", Duration::from_secs(5));
    assert!(a["result"].is_object(), "{a}");
    assert!(client.alive());

    // The client ends the session: the bridge ends with it.
    assert_eq!(client.close_and_wait(Duration::from_secs(5)), Some(0));
}

#[test]
fn without_a_drop_the_bridge_adds_nothing() {
    // Negative control: a node that stays answers every call; no error of the bridge's appears.
    let path = socket("steady");
    let node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    for _ in 0..20 {
        let a = client.call("echo", Duration::from_secs(5));
        assert_eq!(a["result"]["connection"], 1, "{a}");
    }
    assert_eq!(node.seen.connections.load(Ordering::SeqCst), 1);
    assert_eq!(client.close_and_wait(Duration::from_secs(5)), Some(0));
}

#[test]
fn a_peer_that_refuses_the_handshake_is_not_given_the_clients_calls() {
    let path = socket("refusing");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    node.stop();
    // Something else now answers at the path: it does not take `initialize`, so the bridge never sends it a call.
    let impostor = FakeNode::start(&path, Mode::Refuse);
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(1500) {
        let a = client.call("echo", Duration::from_secs(2));
        assert!(unavailable(&a), "{a}");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(impostor.seen.initializes.load(Ordering::SeqCst) >= 1);
    assert!(client.alive());
    // A node that does take it is used as soon as it appears.
    drop(impostor);
    let _node = FakeNode::start(&path, Mode::Accept);
    let t = Instant::now();
    loop {
        let a = client.call("echo", Duration::from_secs(5));
        if !unavailable(&a) {
            assert!(a["result"].is_object(), "{a}");
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(8), "never reconnected");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_bridge_ends_with_its_client_even_while_the_node_is_away() {
    let path = socket("away");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    node.stop();
    let a = client.call("echo", Duration::from_secs(2));
    assert!(unavailable(&a), "{a}");
    assert_eq!(client.close_and_wait(Duration::from_secs(5)), Some(0));
}

#[test]
fn a_bridge_that_cannot_connect_at_the_start_still_fails_as_before() {
    let path = socket("none");
    let out = Command::new(AGORA)
        .args(["mesh", "stdio", "--socket"])
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn the_bridge_ends_when_its_client_ends_in_the_middle_of_a_reconnect() {
    // The new connection is accepted and its `initialize` is never answered. The client's input ends meanwhile: the bridge must
    // not wait on that connection (or install it after the end) and ends within moments.
    let path = socket("midhandshake");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    node.stop();
    let held = FakeNode::start(&path, Mode::Withhold);
    wait_for("the bridge is waiting for the answer", || {
        held.seen.initializes.load(Ordering::SeqCst) == 1
    });
    let t = Instant::now();
    assert_eq!(client.close_and_wait(Duration::from_secs(2)), Some(0));
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn bytes_the_node_sends_behind_the_handshake_answer_reach_the_client() {
    let path = socket("chatty");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    node.stop();
    let _node = FakeNode::start(&path, Mode::Chatty);
    let a = client
        .answer(Duration::from_secs(8))
        .expect("the node's own line");
    assert_eq!(a["method"], "notifications/chatty", "{a}");
}

#[test]
fn the_bridge_ends_when_the_clients_output_is_gone() {
    let path = socket("deaf");
    let _node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::spawn(&path, false);
    // Its input stays open and idle; the answer to this has nowhere to go.
    client.request("initialize", json!({"protocolVersion":"2025-03-26"}));
    let t = Instant::now();
    loop {
        if client.child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(3), "the bridge lingers");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn while_disconnected_a_batch_gets_a_batch_of_errors_and_notifications_get_nothing() {
    let path = socket("batch");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    client.initialize();
    node.stop();
    // Wait until the bridge knows it is disconnected.
    let a = client.call("echo", Duration::from_secs(2));
    assert!(unavailable(&a), "{a}");

    client.send(&json!([
        {"jsonrpc":"2.0","id":100,"method":"tools/call","params":{"name":"echo"}},
        {"jsonrpc":"2.0","method":"notifications/cancelled","params":{}},
        {"jsonrpc":"2.0","id":101,"method":"tools/list"},
    ]));
    let a = client
        .answer(Duration::from_secs(2))
        .expect("a batch answer");
    let items = a.as_array().expect("an array");
    assert_eq!(items.len(), 2, "{a}");
    assert_eq!(items[0]["id"], 100);
    assert_eq!(items[1]["id"], 101);
    assert!(items.iter().all(unavailable));

    // Notifications, alone or in a batch, are not answered: the next line the client reads answers the next request.
    client.send(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}));
    client.send(&json!([{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}]));
    let a = client.call("echo", Duration::from_secs(2));
    assert!(unavailable(&a), "{a}");
}

/// #981: a client whose conversation has no batches (2025-06-18) gets the requests of a batch it sent while the node was away
/// answered one line each, since it does not read a batch.
#[test]
fn while_disconnected_a_batch_without_batching_gets_one_answer_per_line() {
    let path = socket("nobatch");
    let mut node = FakeNode::start(&path, Mode::Accept);
    let mut client = Client::start(&path);
    let id = client.request("initialize", json!({"protocolVersion":"2025-06-18"}));
    assert_eq!(
        client.answer(Duration::from_secs(5)).expect("initialize")["id"],
        id
    );
    node.stop();
    let a = client.call("echo", Duration::from_secs(2));
    assert!(unavailable(&a), "{a}");
    client.send(&json!([
        {"jsonrpc":"2.0","id":100,"method":"tools/call","params":{"name":"echo"}},
        {"jsonrpc":"2.0","id":101,"method":"tools/list"},
    ]));
    for id in [100, 101] {
        let a = client.answer(Duration::from_secs(2)).expect("an answer");
        assert_eq!(a["id"], id, "{a}");
        assert!(unavailable(&a), "{a}");
    }
}

/// A real node endpoint (the node's mesh server) with one agent, for the bridge to carry a conversation to.
struct RealNode {
    server: agora_node::mesh_server::MeshServer,
    dir: PathBuf,
    socket: PathBuf,
}

impl RealNode {
    fn start(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("agora-mbr-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let server =
            agora_node::mesh_server::MeshServer::new(&dir, agora_mesh::broker::Limits::default())
                .unwrap();
        let socket = server
            .register(agora_mesh::broker::AgentInfo {
                id: agora_core::AgentId::new("a1"),
                kind: "fixture".into(),
                node: "n".into(),
                project: "p".into(),
                label: "a1".into(),
                worktree: String::new(),
                state: "idle".into(),
            })
            .unwrap();
        Self {
            server,
            dir,
            socket,
        }
    }
}

impl Drop for RealNode {
    fn drop(&mut self) {
        self.server.unregister(&agora_core::AgentId::new("a1"));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// #981 and #984 end to end, through `agora mesh stdio` to the node's own endpoint: a line that is not JSON is answered -32700 with
/// id null, and after a 2025-03-26 handshake a batch of `ping` and `tools/list` is answered with an array of both.
#[test]
fn through_the_bridge_bad_json_and_a_batch_are_answered_by_the_node() {
    let node = RealNode::start("e2e");
    let mut client = Client::start(&node.socket);
    client.initialize();
    {
        let stdin = client.stdin.as_mut().unwrap();
        writeln!(stdin, r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"inbox","arguments":{{"#).unwrap();
        stdin.flush().unwrap();
    }
    let e = client
        .answer(Duration::from_secs(5))
        .expect("a parse error");
    assert_eq!(e["error"]["code"], -32700, "{e}");
    assert_eq!(e["id"], Value::Null, "{e}");
    client.send(&json!([
        {"jsonrpc":"2.0","id":40,"method":"ping"},
        {"jsonrpc":"2.0","id":41,"method":"tools/list"},
    ]));
    let b = client
        .answer(Duration::from_secs(5))
        .expect("a batch answer");
    let items = b.as_array().unwrap_or_else(|| panic!("not an array: {b}"));
    assert_eq!(items.len(), 2, "{b}");
    assert_eq!(items[0]["id"], 40, "{b}");
    assert_eq!(items[1]["id"], 41, "{b}");
    assert!(items[1]["result"]["tools"].is_array(), "{b}");
    let id = client.request("ping", json!({}));
    assert_eq!(
        client.answer(Duration::from_secs(5)).expect("ping")["id"],
        id
    );
    assert_eq!(client.close_and_wait(Duration::from_secs(5)), Some(0));
}

/// #981 end to end: under 2025-06-18 (no batches) the node refuses each request of a batch with -32600, and nothing is dropped.
#[test]
fn through_the_bridge_a_batch_without_batching_is_refused_per_request() {
    let node = RealNode::start("e2e-nobatch");
    let mut client = Client::start(&node.socket);
    let id = client.request("initialize", json!({"protocolVersion":"2025-06-18"}));
    assert_eq!(
        client.answer(Duration::from_secs(5)).expect("initialize")["id"],
        id
    );
    client.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    client.send(&json!([
        {"jsonrpc":"2.0","id":40,"method":"ping"},
        {"jsonrpc":"2.0","id":41,"method":"tools/list"},
    ]));
    for id in [40, 41] {
        let e = client.answer(Duration::from_secs(5)).expect("an answer");
        assert_eq!(e["id"], id, "{e}");
        assert_eq!(e["error"]["code"], -32600, "{e}");
    }
    assert_eq!(client.close_and_wait(Duration::from_secs(5)), Some(0));
}
