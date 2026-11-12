//! A transport leaving must not leave its asks in the broker of a still-running agent.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use agora_core::AgentId;
use agora_mesh::broker::{AgentInfo, AskStatus, Limits};
use agora_node::mesh_server::MeshServer;
use agora_platform::LocalStream;
use serde_json::{json, Value};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "agora-asks-{}-{}",
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

    fn ask(&mut self, question: &str) {
        self.send(
            json!({"jsonrpc":"2.0", "id":2, "method":"tools/call", "params":{
                "name":"ask", "arguments":{"to":"b", "question":question, "timeout_s":600}
            }}),
        );
    }

    fn reply(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.writer.shutdown();
    }
}

fn agent(id: &str) -> AgentInfo {
    AgentInfo {
        id: AgentId::new(id),
        kind: "fixture".into(),
        node: "local".into(),
        project: "p".into(),
        label: id.into(),
        worktree: String::new(),
        state: "idle".into(),
    }
}

fn wait(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < until, "MCP ask lifecycle did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn received(server: &MeshServer, question: &str) -> String {
    let mut found = None;
    wait(|| {
        found = server
            .mesh()
            .lock()
            .unwrap()
            .inbox(&AgentId::new("b"), None, 1000)
            .unwrap()
            .iter()
            .find(|m| m.body == question)
            .map(|m| m.id.clone());
        found.is_some()
    });
    found.unwrap()
}

#[test]
fn closing_one_mcp_connection_cancels_its_asks_and_preserves_the_other_connection() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let directory = Directory::new();
    let server = MeshServer::new(&directory.0.join("m"), Limits::default()).unwrap();
    let path = server.register(agent("a")).unwrap();
    server.mesh().lock().unwrap().register(agent("b"));

    let mut surviving = Client::connect(&path);
    surviving.ask("keep this connection");
    let kept = received(&server, "keep this connection");
    for i in 0..8 {
        let question = format!("abandoned connection {i}");
        let mut leaving = Client::connect(&path);
        leaving.ask(&question);
        let abandoned = received(&server, &question);
        drop(leaving);
        // now=0 cannot expire a live request: TimedOut here means its record was actually removed.
        wait(|| server.mesh().lock().unwrap().ask_status(&abandoned, 0) == AskStatus::TimedOut);
        let mesh = server.mesh();
        let mut mesh = mesh.lock().unwrap();
        assert_eq!(mesh.ask_status(&kept, 0), AskStatus::Pending);
        mesh.send_message(
            &AgentId::new("b"),
            "a",
            "too late",
            &[],
            Some(&abandoned),
            0,
        )
        .unwrap();
        assert_eq!(mesh.ask_status(&abandoned, 0), AskStatus::TimedOut);
        assert!(server.is_registered(&AgentId::new("a")));
    }
    server
        .mesh()
        .lock()
        .unwrap()
        .send_message(&AgentId::new("b"), "a", "kept answer", &[], Some(&kept), 0)
        .unwrap();
    let reply = surviving.reply();
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap()["reply"],
        "kept answer"
    );
    drop(surviving);
    drop(server);
    drop(directory);
}
