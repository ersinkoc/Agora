//! ADR-0039: the node renews an agent's shared-board leases while the agent runs. A real mesh server and a running
//! `agora-fake-agent` process (the node takes it off the mesh when it exits, as the daemon does); the hub's board is a second broker
//! behind the node's `RemoteLink`, with the board's own lease rules and clock, and no give-back when an agent ends, so only the TTL
//! and the node's renewals decide how long a lease lasts. Another machine's agent (`n2/b`) probes the path on that board.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, Limits, Mesh};
use agora_node::mesh_server::{BoardReply, MeshServer, RemoteLink};
use agora_platform::LocalStream;
use serde_json::{json, Value};

const PATH: &str = "src/a.rs";
const TTL_S: u64 = 4;

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn fake_agent() -> PathBuf {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let ok = Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success();
        assert!(ok, "building agora-fake-agent failed");
    });
    p
}

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "agora-lr-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture: {e}"),
            }
        }
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The hub's board for project `p`, seen by node `n1`: every call is run on one broker as `n1/<agent>`, as the hub names a node's
/// agent. While `online` is false the hub cannot be reached.
struct Board {
    mesh: Mutex<Mesh>,
    online: AtomicBool,
    /// Every call that reached the board: (caller, tool, arguments).
    calls: Mutex<Vec<(String, String, Value)>>,
}

impl Board {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            mesh: Mutex::new(Mesh::new(Limits::default())),
            online: AtomicBool::new(true),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn call(&self, agent: &str, tool: &str, args: &Value) -> Value {
        let id = AgentId::new(agent);
        let now = now_ms();
        let mut m = self.mesh.lock().unwrap();
        if m.agent_info(&id).is_none() {
            m.join(
                AgentInfo {
                    id: id.clone(),
                    kind: "fixture".into(),
                    node: agent.split('/').next().unwrap_or_default().into(),
                    project: "p".into(),
                    label: agent.into(),
                    worktree: String::new(),
                    state: "working".into(),
                },
                now,
            );
        }
        m.expire_leases(now);
        self.calls
            .lock()
            .unwrap()
            .push((agent.to_string(), tool.to_string(), args.clone()));
        agora_mesh::mcp::board_tool(&id, tool, args, &mut m, now).expect("a board tool")
    }

    /// Another machine's agent asks for the path: `Ok(lease id)` when granted, `Err(error code)` when refused.
    fn probe(&self) -> Result<String, String> {
        let r = self.call(
            "n2/b",
            "lease_acquire",
            &json!({"paths": [PATH], "ttl_s": 60}),
        );
        let body = text(&r);
        if r["isError"] == true {
            Err(body["error"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_string())
        } else {
            Ok(body["lease_id"].as_str().unwrap().to_string())
        }
    }

    /// How many `lease_acquire` calls `n1/a` made after the first `skip` calls on the board.
    fn acquires_by_a(&self, skip: usize) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .skip(skip)
            .filter(|(who, tool, _)| who == "n1/a" && tool == "lease_acquire")
            .count()
    }

    fn calls_so_far(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl RemoteLink for Board {
    fn post(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &[String],
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        done(Err("node.offline".into()));
    }

    fn board(
        &self,
        from_agent: &str,
        project: &str,
        tool: &str,
        arguments_json: &str,
        done: Box<dyn FnOnce(Result<BoardReply, String>) + Send>,
    ) {
        if !self.online.load(Ordering::SeqCst) {
            done(Err("node.offline".into()));
            return;
        }
        assert_eq!(project, "p");
        let args: Value = serde_json::from_str(arguments_json).unwrap();
        let result = self.call(&format!("n1/{from_agent}"), tool, &args);
        done(Ok(BoardReply {
            shared: true,
            result_json: result.to_string(),
        }));
    }
}

fn text(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

struct Client {
    writer: LocalStream,
    reader: BufReader<LocalStream>,
}

impl Client {
    fn connect(path: &std::path::Path) -> Self {
        let writer = agora_platform::local_connect(path).unwrap();
        let reader = BufReader::new(writer.try_clone().unwrap());
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut client = Self { writer, reader };
        client.send(json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{}}));
        assert_eq!(client.reply()["id"], 1);
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.writer, "{message}").unwrap();
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// A tool call as the agent; its result's JSON and whether it is an error.
    fn tool(&mut self, name: &str, arguments: Value) -> (Value, bool) {
        self.send(json!({"jsonrpc":"2.0", "id":2, "method":"tools/call",
            "params":{"name": name, "arguments": arguments}}));
        let reply = self.reply();
        let result = &reply["result"];
        (text(result), result["isError"] == true)
    }
}

/// A node with one running agent `a` of project `p` (a fake agent process), its MCP connection, and the hub's board.
struct Scene {
    _server: Arc<MeshServer>,
    board: Arc<Board>,
    /// The fake agent's input: it runs until this closes.
    agent: Option<ChildStdin>,
    exited: Arc<AtomicBool>,
    client: Client,
    _dir: Directory,
}

impl Scene {
    fn new() -> Self {
        let dir = Directory::new();
        let server = Arc::new(MeshServer::new(&dir.0.join("m"), Limits::default()).unwrap());
        let board = Board::new();
        server.set_remote(board.clone() as Arc<dyn RemoteLink>);
        let id = AgentId::new("a");
        let socket = server
            .register(AgentInfo {
                id: id.clone(),
                kind: "acp-fake".into(),
                node: "n1".into(),
                project: "p".into(),
                label: "a".into(),
                worktree: String::new(),
                state: "working".into(),
            })
            .unwrap();
        let mut child = Command::new(fake_agent())
            .arg("--acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let agent = child.stdin.take();
        // The node's supervision: the agent's process ending takes it off the mesh (`unregister_because`, as the daemon does).
        let exited = Arc::new(AtomicBool::new(false));
        {
            let (server, exited) = (Arc::clone(&server), Arc::clone(&exited));
            std::thread::spawn(move || {
                let _ = child.wait();
                server.unregister_because(&id, "finished");
                exited.store(true, Ordering::SeqCst);
            });
        }
        let client = Client::connect(&socket);
        Self {
            _server: server,
            board,
            agent,
            exited,
            client,
            _dir: dir,
        }
    }

    fn acquire(&mut self) -> String {
        let (answer, failed) = self
            .client
            .tool("lease_acquire", json!({"paths": [PATH], "ttl_s": TTL_S}));
        assert!(!failed, "{answer}");
        answer["lease_id"].as_str().unwrap().to_string()
    }

    /// The agent's process ends (its input closes) and the node takes it off the mesh.
    fn end_agent(&mut self) {
        drop(self.agent.take());
        let until = Instant::now() + Duration::from_secs(20);
        while !self.exited.load(Ordering::SeqCst) {
            assert!(Instant::now() < until, "the fake agent did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        // The fake agent ends with its input; the watcher then takes it off the mesh.
        drop(self.agent.take());
    }
}

/// The lease outlives its TTL several times over while the agent runs (another machine's agent is refused all along), and once the
/// agent exits it ends within one TTL.
#[test]
fn a_shared_lease_outlives_its_ttl_while_the_agent_runs_and_lapses_after_it_exits() {
    let mut s = Scene::new();
    let start = Instant::now();
    s.acquire();
    let first = s.board.calls_so_far();
    while start.elapsed() < Duration::from_secs(TTL_S * 3) {
        assert_eq!(
            s.board.probe(),
            Err("lease.held".to_string()),
            "the lease ended at {:?} while its agent runs",
            start.elapsed()
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    let renewals = s.board.acquires_by_a(first);
    assert!(renewals >= 3, "renewed {renewals} times in three TTLs");
    // At half the TTL, not faster.
    assert!(renewals <= 8, "renewed {renewals} times in three TTLs");

    s.end_agent();
    let ended = Instant::now();
    let after = s.board.calls_so_far();
    let granted = loop {
        match s.board.probe() {
            Ok(_) => break ended.elapsed(),
            Err(code) => assert_eq!(code, "lease.held"),
        }
        assert!(
            ended.elapsed() < Duration::from_secs(TTL_S + 2),
            "the lease of an agent that exited was still held after its TTL"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(granted <= Duration::from_secs(TTL_S + 2), "{granted:?}");
    assert_eq!(
        s.board.acquires_by_a(after),
        0,
        "renewed after the agent exited"
    );
}

/// The agent releases a renewed lease with the id it was given (the board's id changed with each renewal), and it is not renewed
/// again.
#[test]
fn the_agents_own_lease_id_still_releases_a_renewed_lease() {
    let mut s = Scene::new();
    let id = s.acquire();
    let first = s.board.calls_so_far();
    std::thread::sleep(Duration::from_millis(TTL_S * 1000 + 500));
    assert!(s.board.acquires_by_a(first) >= 1, "not renewed");
    let (answer, failed) = s.client.tool("lease_release", json!({"lease_id": id}));
    assert!(!failed, "{answer}");
    assert_eq!(answer, json!({"ok": true}));
    assert!(s.board.probe().is_ok(), "the released path is still held");
    let after = s.board.calls_so_far();
    std::thread::sleep(Duration::from_millis(TTL_S * 1000));
    assert_eq!(s.board.acquires_by_a(after), 0, "renewed after its release");
    // An id the agent was never given is still refused.
    let (answer, failed) = s.client.tool("lease_release", json!({"lease_id": "l-404"}));
    assert!(failed);
    assert_eq!(answer["error"]["code"], "lease.not_found");
}

/// A node that cannot reach the hub cannot renew: the board ends the lease at its TTL although the agent still runs, and when the
/// hub is back the node does not take it back.
#[test]
fn a_node_cut_off_from_the_hub_loses_the_lease_at_its_ttl_and_does_not_take_it_back() {
    let mut s = Scene::new();
    s.acquire();
    s.board.online.store(false, Ordering::SeqCst);
    let cut = Instant::now();
    std::thread::sleep(Duration::from_millis(TTL_S * 1000 + 500));
    let back = s.board.calls_so_far();
    s.board.online.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(2_000));
    assert_eq!(
        s.board.acquires_by_a(back),
        0,
        "the node took the lease back after {:?} offline",
        cut.elapsed()
    );
    assert!(
        s.board.probe().is_ok(),
        "the lease outlived its TTL with no renewal"
    );
    assert!(!s.exited.load(Ordering::SeqCst), "the agent kept running");
}
