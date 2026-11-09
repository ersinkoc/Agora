//! The ACP bridge (L2) against the fake ACP agent: the node's side speaks the adapter protocol over in-memory pipes, the agent's side
//! is a real child process (this test binary, re-executed as the fake agent). No real agent, account or token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[test]
fn acp_child() {
    if std::env::var_os("AGORA_TEST_ACP").is_none() {
        return;
    }
    println!(); // end the harness's unfinished "test acp_child ... " line; JSON starts on its own
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

struct ChanReader(Receiver<Vec<u8>>, Vec<u8>);
impl Read for ChanReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1.is_empty() {
            match self.0.recv() {
                Ok(b) => self.1 = b,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.1.len());
        buf[..n].copy_from_slice(&self.1[..n]);
        self.1.drain(..n);
        Ok(n)
    }
}

#[derive(Clone)]
struct Out(Arc<Mutex<Vec<u8>>>);
impl Write for Out {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Node {
    to_bridge: Sender<Vec<u8>>,
    seen: Out,
    done: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    next: u64,
}

impl Node {
    fn start() -> Self {
        std::env::set_var("AGORA_TEST_ACP", "1");
        let (tx, rx) = channel();
        let out = Out(Arc::default());
        let agent: Vec<String> = [
            std::env::current_exe().unwrap().to_string_lossy().as_ref(),
            "--exact",
            "acp_child",
            "--nocapture",
            "--test-threads=1",
        ]
        .map(String::from)
        .to_vec();
        let o = out.clone();
        let done = std::thread::spawn(move || {
            agora_adapters::acp::run(&agent, ChanReader(rx, Vec::new()), o)
        });
        Self {
            to_bridge: tx,
            seen: out,
            done: Some(done),
            next: 0,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.to_bridge
            .send(
                format!(
                    "{}\n",
                    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
                )
                .into_bytes(),
            )
            .unwrap();
        self.wait(|m| m.get("id") == Some(&json!(id)))
    }

    fn messages(&self) -> Vec<Value> {
        String::from_utf8_lossy(&self.seen.0.lock().unwrap())
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn wait(&self, mut want: impl FnMut(&Value) -> bool) -> Value {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(m) = self.messages().into_iter().find(|m| want(m)) {
                return m;
            }
            assert!(
                Instant::now() < end,
                "timed out; saw {:#?}",
                self.messages()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn state(&self, s: &str) -> bool {
        self.messages()
            .iter()
            .any(|m| m["method"] == "state/update" && m["params"]["state"] == s)
    }
}

fn chats(n: &Node) -> Vec<Value> {
    n.messages()
        .into_iter()
        .filter(|m| m["method"] == "chat/append")
        .map(|m| m["params"].clone())
        .collect()
}

#[test]
fn an_acp_agent_looks_like_an_adapter_to_the_node() {
    let mut n = Node::start();
    let init = n.call("initialize", json!({"protocolGeneration": 1}));
    assert_eq!(init["result"]["adapterName"], "acp-bridge");
    let started = n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    assert_eq!(started["result"]["sessionId"], "acp-s1");
    n.wait(|m| m["method"] == "state/update" && m["params"]["state"] == "IDLE");

    // A turn that plans, edits and talks: a plan, a tool call, a diff, and the chunks joined into one text.
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "plan"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "hello world");
    let c = chats(&n);
    assert!(
        c.iter()
            .any(|p| p["plan"]["steps"] == json!(["read the code", "fix the bug"])),
        "{c:#?}"
    );
    assert!(
        c.iter().any(|p| p["toolCall"]["tool"] == "edit a.rs"),
        "{c:#?}"
    );
    let diff = c.iter().find(|p| p.get("diff").is_some()).expect("a diff");
    assert_eq!(diff["diff"]["path"], "a.rs");
    assert_eq!(
        diff["diff"]["unifiedDiff"],
        "--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"
    );
    let working = n
        .messages()
        .iter()
        .position(|m| m["params"]["state"] == "WORKING")
        .expect("working");
    let idle_after = n
        .messages()
        .iter()
        .rposition(|m| m["params"]["state"] == "IDLE")
        .expect("idle");
    assert!(working < idle_after, "the turn ends idle");

    // Another prompt: a permission request becomes an approval; answering it reaches the agent's own option.
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "ask"}),
    );
    let ask = n.wait(|m| m["method"] == "approval/request");
    assert_eq!(
        (
            ask["params"]["action"].as_str(),
            ask["params"]["detail"].as_str()
        ),
        (Some("run_command"), Some("{\"command\":\"ls\"}"))
    );
    assert!(n.state("BLOCKED"));
    let id = ask["params"]["approvalId"].as_str().unwrap().to_string();
    n.call(
        "approval/resolve",
        json!({"sessionId": "acp-s1", "approvalId": id, "approved": true}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "allowed");

    // And a refusal picks the reject option.
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "ask"}),
    );
    let again =
        n.wait(|m| m["method"] == "approval/request" && m["params"]["approvalId"] != json!(id));
    n.call("approval/resolve", json!({"sessionId": "acp-s1", "approvalId": again["params"]["approvalId"], "approved": false}));
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "denied");

    // Stopping ends the bridge and its agent.
    n.call(
        "session/stop",
        json!({"sessionId": "acp-s1", "reason": "user"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_ok());
}

#[test]
fn the_node_is_told_the_bridge_is_ready_only_after_the_agent_finished_initializing() {
    std::env::set_var("AGORA_FAKE_ACP_INIT_DELAY_MS", "400");
    let n = Node::start();
    n.to_bridge
        .send(
            format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
            )
            .into_bytes(),
        )
        .unwrap();
    // The agent is still initializing: a strict ACP agent would refuse `session/new` now, so the node must not be told to start one.
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        !n.messages().iter().any(|m| m.get("id") == Some(&json!(1))),
        "answered early: {:?}",
        n.messages()
    );
    let reply = n.wait(|m| m.get("id") == Some(&json!(1)));
    assert_eq!(reply["result"]["adapterName"], "acp-bridge");
    std::env::remove_var("AGORA_FAKE_ACP_INIT_DELAY_MS");
}

#[test]
fn an_agent_that_dies_ends_the_bridge_so_the_node_sees_the_adapter_exit() {
    let mut n = Node::start();
    n.call("initialize", json!({}));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "crash"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_err());
}
