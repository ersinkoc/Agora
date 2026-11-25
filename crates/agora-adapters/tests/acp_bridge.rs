//! The ACP bridge (L2) against the fake ACP agent: the node's side speaks the adapter protocol over in-memory pipes, the agent's side
//! is a real child process (this test binary, re-executed as the fake agent). No real agent, account or token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const LARGE_PROMPT_BYTES: usize = 256 * 1024;

#[test]
fn acp_child() {
    // Only as the bridge's child: the harness running every test sees the variable too once another test set it, and would
    // otherwise play the agent on its own stdin.
    if std::env::var_os("AGORA_TEST_ACP").is_none() || !std::env::args().any(|a| a == "acp_child") {
        return;
    }
    println!(); // end the harness's unfinished "test acp_child ... " line; JSON starts on its own
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// The fake agent that dies before `initialize` (a broken plugin), for `Node::start_with`.
#[test]
fn acp_child_fails_at_start() {
    // Only as the bridge's child: the harness running every test sees the variable too (other tests set it), and this one exits.
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_fails_at_start")
    {
        return;
    }
    let code = agora_fake_agent::acp::fail_at_start(&mut std::io::stderr().lock());
    std::process::exit(code);
}

/// An agent that sends an oversized line instead of answering initialize, then leaves stdout open.
#[test]
fn acp_child_sends_an_oversized_line() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_sends_an_oversized_line")
    {
        return;
    }
    println!();
    let mut input = std::io::stdin().lock();
    let mut request = String::new();
    input.read_line(&mut request).unwrap();
    let mut output = std::io::stdout().lock();
    let mut bytes = io::repeat(b' ').take((agora_adapters::host::MAX_LINE + 1) as u64);
    if io::copy(&mut bytes, &mut output).is_ok() && output.flush().is_ok() {
        let _ = io::copy(&mut input, &mut io::sink());
    }
}

/// An agent that emits progress while it is still reading a large prompt.
#[test]
fn acp_child_streams_before_reading_large_prompt() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_streams_before_reading_large_prompt")
    {
        return;
    }
    println!();
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    for (method, result) in [
        (
            "initialize",
            json!({"protocolVersion": 1, "agentCapabilities": {}}),
        ),
        ("session/new", json!({"sessionId": "acp-s1"})),
    ] {
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], method);
        let reply = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
        writeln!(output, "{reply}").unwrap();
        output.flush().unwrap();
    }

    // Read only one byte before emitting enough complete updates to fill the bridge's
    // input queue and OS pipe. The rest of the prompt remains outstanding during the burst.
    let mut first = [0];
    input.read_exact(&mut first).unwrap();
    let mut update = |text: &str| {
        let message = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": "acp-s1", "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text},
            }},
        });
        writeln!(output, "{message}").unwrap();
        output.flush().unwrap();
    };
    let progress = "p".repeat(16 * 1024);
    for _ in 0..16 {
        update(&progress);
    }
    let mut line = first.to_vec();
    input.read_until(b'\n', &mut line).unwrap();
    let prompt: Value = serde_json::from_slice(&line).unwrap();
    assert_eq!(prompt["method"], "session/prompt");
    assert_eq!(
        prompt
            .pointer("/params/prompt/0/text")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        LARGE_PROMPT_BYTES,
    );
    update("fixture prompt received");
    let reply = json!({"jsonrpc": "2.0", "id": prompt["id"], "result": {"stopReason": "end_turn"}});
    writeln!(output, "{reply}").unwrap();
    output.flush().unwrap();
    let _ = io::copy(&mut input, &mut io::sink());
}

/// The fake agent that refuses `session/new`.
#[test]
fn acp_child_refuses_session() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_refuses_session")
    {
        return;
    }
    println!();
    // Set in this child process only, so no other test's agent refuses.
    std::env::set_var("AGORA_FAKE_ACP_REFUSE_SESSION", "1");
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// The fake agent that says it takes no stdio MCP servers (`mcpCapabilities.stdio: false`).
#[test]
fn acp_child_no_stdio_mcp() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_no_stdio_mcp")
    {
        return;
    }
    println!();
    std::env::set_var("AGORA_FAKE_ACP_NO_STDIO_MCP", "1");
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// The fake agent that is slow to initialize.
#[test]
fn acp_child_init_delay() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_init_delay")
    {
        return;
    }
    println!();
    std::env::set_var("AGORA_FAKE_ACP_INIT_DELAY_MS", "400");
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// The fake agent that offers a bypass mode and refuses a model switch.
#[test]
fn acp_child_bypass() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_bypass")
    {
        return;
    }
    println!();
    std::env::set_var("AGORA_FAKE_ACP_BYPASS_MODE", "1");
    std::env::set_var("AGORA_FAKE_ACP_REFUSE_MODEL", "1");
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
    /// A delayed notification delivered after `turn_end` takes its starting offset, before the prompt is sent.
    before_prompt: Option<Value>,
}

impl Node {
    fn start() -> Self {
        Self::start_with("acp_child")
    }

    /// The bridge with this test function of the binary as its agent.
    fn start_with(child: &str) -> Self {
        std::env::set_var("AGORA_TEST_ACP", "1");
        let (tx, rx) = channel();
        let out = Out(Arc::default());
        let agent: Vec<String> = [
            std::env::current_exe().unwrap().to_string_lossy().as_ref(),
            "--exact",
            child,
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
            before_prompt: None,
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        if method == "session/prompt" {
            if let Some(notification) = self.before_prompt.take() {
                writeln!(self.seen, "{notification}").unwrap();
            }
        }
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
    n.wait(|m| m["method"] == "state/update" && m["params"]["state"] == "AGENT_STATE_IDLE");

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
        .position(|m| m["params"]["state"] == "AGENT_STATE_WORKING")
        .expect("working");
    let idle_after = n
        .messages()
        .iter()
        .rposition(|m| m["params"]["state"] == "AGENT_STATE_IDLE")
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
    assert!(n.state("AGENT_STATE_BLOCKED"));
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

/// ADR-0111 build step 3: the bridge declares `chat.cuts` (it reports every place it cuts a chat item's text) to a node that reads
/// rich items, as every node since ADR-0090 does; one that does not is served as before (`a_node_without_rich_chat_gets_the_old_items_byte_for_byte`).
#[test]
fn the_bridge_declares_that_it_reports_its_cuts() {
    let mut n = Node::start();
    let init = n.call(
        "initialize",
        json!({"protocolGeneration": 1, "nodeCapabilities": ["chat", "chat.rich"]}),
    );
    let caps = init["result"]["capabilities"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        caps.contains(&json!(agora_adapters::host::CHAT_CUTS)),
        "{caps:?}"
    );
}

#[test]
fn the_node_is_told_the_bridge_is_ready_only_after_the_agent_finished_initializing() {
    let n = Node::start_with("acp_child_init_delay");
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

#[test]
fn an_agent_can_stream_updates_while_a_large_prompt_is_written() {
    let mut n = Node::start_with("acp_child_streams_before_reading_large_prompt");
    assert_eq!(
        n.call("initialize", json!({}))["result"]["adapterName"],
        "acp-bridge"
    );
    let started = n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    assert_eq!(started["result"]["sessionId"], "acp-s1");
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "x".repeat(LARGE_PROMPT_BYTES)}),
    );
    n.wait(|m| {
        m["method"] == "chat/append" && m["params"]["text"]["text"] == "fixture prompt received"
    });
    assert!(chats(&n).iter().any(|m| {
        m["text"]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("ppp"))
    }));
    n.call(
        "session/stop",
        json!({"sessionId": "acp-s1", "reason": "user"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_ok());
}

#[test]
fn an_oversized_agent_line_ends_initialize_with_the_agent_exit_reason() {
    let mut n = Node::start_with("acp_child_sends_an_oversized_line");
    let reply = n.call("initialize", json!({}));
    assert_eq!(reply["error"]["data"]["code"], "adapter.agent_exited");
    assert_eq!(reply["error"]["data"]["params"]["stage"], "initialize");
    assert_eq!(reply["error"]["data"]["params"]["stderr_tail"], "");
    let bridge = n.done.take().unwrap();
    drop(n);
    let error = bridge.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn an_oversized_node_line_ends_the_bridge_while_input_is_open() {
    let mut n = Node::start();
    assert_eq!(
        n.call("initialize", json!({}))["result"]["adapterName"],
        "acp-bridge"
    );
    n.to_bridge
        .send(vec![b' '; agora_adapters::host::MAX_LINE + 1])
        .unwrap();
    let bridge = n.done.take().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while !bridge.is_finished() {
        assert!(Instant::now() < end, "bridge did not close");
        std::thread::sleep(Duration::from_millis(20));
    }
    let error = bridge.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

fn started() -> Node {
    let mut n = Node::start();
    n.call("initialize", json!({}));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    n
}

fn logs(n: &Node) -> Vec<Value> {
    n.messages()
        .into_iter()
        .filter(|m| m["method"] == "log")
        .map(|m| serde_json::from_str(m["params"]["message"].as_str().unwrap()).unwrap())
        .collect()
}

/// Runs one turn and returns the reason its closing `IDLE` carries (empty for a turn that ended normally).
fn turn_end(n: &mut Node, prompt: &str) -> String {
    let before = n.messages().len();
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": prompt}),
    );
    let idle =
        |m: &Value| m["method"] == "state/update" && m["params"]["state"] == "AGENT_STATE_IDLE";
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let msgs = n.messages();
        // session/start's initial IDLE can arrive after `before`. Only an IDLE after this prompt's WORKING ends its turn.
        if let Some(working) = msgs[before..].iter().position(|m| {
            m["method"] == "state/update" && m["params"]["state"] == "AGENT_STATE_WORKING"
        }) {
            if let Some(m) = msgs[before + working + 1..].iter().find(|m| idle(m)) {
                return m["params"]["message"].as_str().unwrap_or("").to_string();
            }
        }
        assert!(
            Instant::now() < end,
            "no idle after {prompt}: {:#?}",
            &msgs[before..]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_turn_that_fails_or_says_nothing_is_not_a_silent_success() {
    let mut n = started();
    assert_eq!(
        turn_end(&mut n, "hi"),
        "",
        "a normal turn carries no reason"
    );
    assert_eq!(turn_end(&mut n, "silent"), "adapter.turn_empty");
    assert_eq!(turn_end(&mut n, "refuse"), "adapter.turn_stopped");
    assert_eq!(turn_end(&mut n, "fail"), "adapter.turn_failed");
    let l = logs(&n);
    assert!(
        l.contains(&json!({"code": "adapter.turn_empty", "params": {}})),
        "{l:#?}"
    );
    assert!(
        l.contains(&json!({"code": "adapter.turn_stopped", "params": {"stop_reason": "refusal"}})),
        "{l:#?}"
    );
    assert!(
        l.contains(&json!({"code": "adapter.turn_failed", "params": {"rpc_code": "-32603"}})),
        "{l:#?}"
    );
    // Only the three turns that need it said anything in the log.
    assert_eq!(l.len(), 3, "{l:#?}");
}

#[test]
fn an_agent_that_exits_mid_turn_is_reported_with_its_exit_code_and_stderr() {
    let mut n = started();
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "crash-loud"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_err());
    let l = logs(&n);
    let gone = l
        .iter()
        .find(|e| e["code"] == "adapter.agent_exited")
        .unwrap_or_else(|| panic!("{l:#?}"));
    assert_eq!(gone["params"]["stage"], "turn");
    assert_eq!(gone["params"]["exit_code"], "4");
    let tail = gone["params"]["stderr_tail"].as_str().unwrap();
    assert!(tail.contains("free usage limit reached"), "{tail}");
    assert!(!tail.contains("fake0123456789"), "a secret leaked: {tail}");
}

#[test]
fn an_agent_that_dies_before_initialize_gets_its_own_reason() {
    let n = Node::start_with("acp_child_fails_at_start");
    n.to_bridge
        .send(
            format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
            )
            .into_bytes(),
        )
        .unwrap();
    let reply = n.wait(|m| m.get("id") == Some(&json!(1)));
    let data = &reply["error"]["data"];
    assert_eq!(data["code"], "adapter.agent_exited", "{reply:#}");
    assert_eq!(data["params"]["stage"], "initialize");
    assert_eq!(data["params"]["exit_code"], "5");
    let tail = data["params"]["stderr_tail"].as_str().unwrap();
    assert!(tail.contains("plugin failed"), "{tail}");
    assert!(tail.contains("TypeError"), "{tail}");
    assert!(!tail.contains("sk-fake"), "a secret leaked: {tail}");
    assert!(
        !tail.contains('\u{1b}'),
        "escape sequences stay out: {tail:?}"
    );
    assert!(tail.len() <= 400 + '…'.len_utf8(), "{tail}");
}

#[test]
fn a_running_cost_becomes_usage_reports_of_what_it_grew_by() {
    let mut n = started();
    let init = n.messages();
    assert!(
        init.iter().any(|m| m["result"]["capabilities"]
            .as_array()
            .is_some_and(|c| c.contains(&json!("usage")))),
        "usage is declared"
    );
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "usage"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "counted");
    let reports: Vec<Value> = n
        .messages()
        .into_iter()
        .filter(|m| m["method"] == "usage/report")
        .map(|m| m["params"].clone())
        .collect();
    assert_eq!(reports.len(), 2, "{reports:#?}");
    assert_eq!(reports[0]["costMicros"], 12300);
    assert_eq!(reports[1]["costMicros"], 7700);
    for r in &reports {
        assert_eq!(r["currency"], "USD");
        assert_eq!(r["model"], "fake/model-1");
        assert_eq!(
            (r["inputTokens"].as_u64(), r["outputTokens"].as_u64()),
            (Some(0), Some(0))
        );
    }
}

#[test]
fn a_replayed_lower_cost_does_not_charge_the_same_session_growth_again() {
    let mut n = started();
    for _ in 0..3 {
        // The fake reports the same cumulative amounts each time: 0.0123 then 0.02 USD.
        turn_end(&mut n, "usage");
    }
    let increments: Vec<u64> = n
        .messages()
        .iter()
        .filter(|m| m["method"] == "usage/report")
        .filter_map(|m| m["params"]["costMicros"].as_u64())
        .collect();
    assert_eq!(increments, [12300, 7700]);
    n.call("shutdown", json!({}));
    n.done.take().unwrap().join().unwrap().unwrap();
}

#[test]
fn the_context_level_and_size_ride_along_with_the_cost() {
    let mut n = started();
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "usage"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "counted");
    for r in n
        .messages()
        .into_iter()
        .filter(|m| m["method"] == "usage/report")
    {
        assert_eq!(r["params"]["contextTokens"], 1200, "{r:#}");
        assert_eq!(r["params"]["contextWindow"], 1_000_000, "{r:#}");
    }
}

fn notices(n: &Node) -> Vec<Value> {
    chats(n)
        .into_iter()
        .filter_map(|c| c.get("notice").cloned())
        .collect()
}

#[test]
fn a_failed_stopped_or_empty_turn_is_a_notice_in_the_conversation() {
    let mut n = started();
    turn_end(&mut n, "hi");
    assert!(notices(&n).is_empty(), "a normal turn has no notice");
    turn_end(&mut n, "silent");
    turn_end(&mut n, "refuse");
    turn_end(&mut n, "fail");
    assert_eq!(
        notices(&n),
        vec![
            json!({"code": "adapter.turn_empty", "params": {}}),
            json!({"code": "adapter.turn_stopped", "params": {"stop_reason": "refusal"}}),
            json!({"code": "adapter.turn_failed", "params": {"rpc_code": "-32603"}}),
        ]
    );
    // A notice is not the agent saying something: the next silent turn is still empty.
    assert_eq!(turn_end(&mut n, "silent"), "adapter.turn_empty");
}

#[test]
fn an_agent_that_exits_mid_turn_leaves_a_notice_with_its_reason() {
    let mut n = started();
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "crash-loud"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_err());
    let all = notices(&n);
    let gone = all
        .iter()
        .find(|e| e["code"] == "adapter.agent_exited")
        .unwrap_or_else(|| panic!("{all:#?}"));
    assert_eq!(gone["params"]["stage"], "turn");
    assert_eq!(gone["params"]["exit_code"], "4");
    let tail = gone["params"]["stderr_tail"].as_str().unwrap();
    assert!(tail.contains("free usage limit reached"), "{tail}");
    assert!(!tail.contains("fake0123456789"), "a secret leaked: {tail}");
}

fn options(n: &Node) -> Vec<Value> {
    n.messages()
        .into_iter()
        .filter(|m| m["method"] == "session/options")
        .map(|m| m["params"].clone())
        .collect()
}

#[test]
fn the_agents_modes_and_models_are_reported_and_can_be_switched() {
    let mut n = Node::start_with("acp_child_bypass");
    let init = n.call("initialize", json!({}));
    let caps = init["result"]["capabilities"].as_array().unwrap().clone();
    assert!(caps.contains(&json!("set_mode")) && caps.contains(&json!("set_model")));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    let first = n.wait(|m| m["method"] == "session/options")["params"].clone();
    assert_eq!(first["currentMode"], "build");
    assert_eq!(first["currentModel"], "fake/model-1");
    let ids = |l: &Value| -> Vec<String> {
        l.as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&first["models"]), ["fake/model-1", "fake/model-2"]);
    assert_eq!(ids(&first["modes"]), ["build", "plan", "bypassPermissions"]);
    let skips: Vec<bool> = first["modes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["skipsPermissions"] == true)
        .collect();
    assert_eq!(
        skips,
        [false, false, true],
        "only the bypass mode is marked"
    );

    // A mode the agent offers: the agent takes it and the new current mode is reported.
    let ok = n.call(
        "session/set_option",
        json!({"sessionId": "acp-s1", "kind": "SESSION_OPTION_KIND_MODE", "id": "plan"}),
    );
    assert!(ok.get("result").is_some(), "{ok:#}");
    let now = options(&n);
    assert_eq!(now.last().unwrap()["currentMode"], "plan");

    // A model the agent offers.
    n.call(
        "session/set_option",
        json!({"sessionId": "acp-s1", "kind": 2, "id": "fake/model-1"}),
    );
    // One it does not offer never reaches it; one it refuses comes back refused. Neither changes what is reported.
    let before = options(&n).len();
    for (kind, id) in [
        ("SESSION_OPTION_KIND_MODEL", "fake/model-9"),
        ("SESSION_OPTION_KIND_MODEL", "fake/model-2"),
        ("SESSION_OPTION_KIND_MODE", "fake/model-1"),
        ("SESSION_OPTION_KIND_UNSPECIFIED", "plan"),
    ] {
        let r = n.call(
            "session/set_option",
            json!({"sessionId": "acp-s1", "kind": kind, "id": id}),
        );
        assert_eq!(
            r["error"]["data"]["code"], "adapter.option_refused",
            "{r:#}"
        );
        assert_eq!(r["error"]["data"]["params"]["id"], id);
    }
    assert_eq!(options(&n).len(), before);
    assert_eq!(options(&n).last().unwrap()["currentModel"], "fake/model-1");

    // The agent switching its own mode is reported too.
    n.call(
        "session/set_option",
        json!({"sessionId": "acp-s1", "kind": "SESSION_OPTION_KIND_MODE", "id": "build"}),
    );
    turn_end(&mut n, "auto");
    assert_eq!(options(&n).last().unwrap()["currentMode"], "plan");
}

#[test]
fn an_agent_that_refuses_the_session_says_so_with_a_code() {
    let mut n = Node::start_with("acp_child_refuses_session");
    n.call("initialize", json!({}));
    let r = n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    assert_eq!(
        r["error"]["data"],
        json!({"code": "adapter.agent_refused", "params": {"stage": "session"}}),
        "{r:#}"
    );
    // The same as a notice, with the agent's code and its message (home folder scrubbed).
    let all = notices(&n);
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0]["code"], "adapter.agent_refused");
    assert_eq!(all[0]["params"]["stage"], "session");
    assert_eq!(all[0]["params"]["rpc_code"], "-32603");
    let detail = all[0]["params"]["detail"].as_str().unwrap();
    assert!(
        detail.contains("ServiceUnavailableError") && !detail.contains("someone"),
        "{detail}"
    );
}

#[test]
fn an_agent_that_died_before_the_node_asked_still_answers_initialize_with_its_reason() {
    let n = Node::start_with("acp_child_fails_at_start");
    // The agent is gone (and reported as `start`) before the node's `initialize` arrives.
    n.wait(|m| m["method"] == "log");
    n.to_bridge
        .send(
            format!(
                "{}\n",
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
            )
            .into_bytes(),
        )
        .unwrap();
    let reply = n.wait(|m| m.get("id") == Some(&json!(1)));
    let data = &reply["error"]["data"];
    assert_eq!(data["code"], "adapter.agent_exited", "{reply:#}");
    assert_eq!(data["params"]["stage"], "initialize");
    assert_eq!(data["params"]["exit_code"], "5");
}

#[test]
fn an_empty_turn_carries_what_the_agent_wrote_to_stderr_during_it() {
    let mut n = started();
    assert_eq!(turn_end(&mut n, "silent"), "adapter.turn_empty");
    assert_eq!(turn_end(&mut n, "silent-loud"), "adapter.turn_empty");
    let all = notices(&n);
    assert_eq!(
        all[0],
        json!({"code": "adapter.turn_empty", "params": {}}),
        "nothing written, nothing added"
    );
    let tail = all[1]["params"]["stderr_tail"]
        .as_str()
        .unwrap_or_else(|| panic!("{all:#?}"));
    assert!(tail.contains("invalid x-api-key (auth)"), "{tail}");
    assert!(
        !tail.contains("/Users/someone") && !tail.contains("sk-fake"),
        "{tail}"
    );
    if let Ok(home) = std::env::var("HOME") {
        assert!(home.len() < 2 || !tail.contains(&home), "{tail}");
    }
    // The next turn starts clean: an empty one without stderr adds nothing.
    turn_end(&mut n, "silent");
    assert_eq!(
        notices(&n)[2],
        json!({"code": "adapter.turn_empty", "params": {}})
    );
}

#[test]
fn a_delayed_startup_idle_cannot_end_the_first_prompt() {
    let mut n = started();
    // session/start may reply before its initial IDLE reaches the node. Force that notification into the prompt's window;
    // the real fake agent still supplies this turn's WORKING, notice and closing IDLE.
    n.before_prompt = Some(json!({
        "jsonrpc": "2.0", "method": "state/update", "params": {"state": "AGENT_STATE_IDLE"}
    }));
    assert_eq!(turn_end(&mut n, "silent"), "adapter.turn_empty");
    assert_eq!(notices(&n)[0]["code"], "adapter.turn_empty");
}

/// The node's `mcp_servers` (the agent's own mesh server) reach the agent's `session/new` as ACP stdio servers.
#[test]
fn the_mesh_server_reaches_the_agents_session() {
    let mesh = json!({"name": "agora-mesh", "command": "/usr/bin/agora",
                      "args": ["mesh", "stdio", "--socket", "/s/agent-1.sock"]});
    let mut n = Node::start();
    n.call("initialize", json!({"protocolGeneration": 1}));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": ".", "mcpServers": [mesh]}),
    );
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "mcp-servers"}),
    );
    let said = n.wait(|m| {
        m["method"] == "chat/append"
            && m["params"]["text"]["text"]
                .as_str()
                .is_some_and(|t| t.starts_with('['))
    });
    let got: Value =
        serde_json::from_str(said["params"]["text"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        got,
        json!([{"name": "agora-mesh", "command": "/usr/bin/agora",
                "args": ["mesh", "stdio", "--socket", "/s/agent-1.sock"], "env": []}])
    );

    // An agent that says it takes no stdio MCP servers gets none.
    let mut n = Node::start_with("acp_child_no_stdio_mcp");
    n.call("initialize", json!({"protocolGeneration": 1}));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": ".", "mcpServers": [mesh]}),
    );
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "mcp-servers"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "[]");
}

fn edge_child_requested(name: &str) -> bool {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    args == ["--exact", name, "--nocapture", "--test-threads=1"]
}

fn edge_child_send(out: &mut impl Write, msg: Value) {
    writeln!(out, "{msg}").unwrap();
    out.flush().unwrap();
}

#[test]
fn acp_edge_noise_child() {
    if !edge_child_requested("acp_edge_noise_child") {
        return;
    }
    let mut input = BufReader::new(io::stdin().lock());
    let mut line = String::new();
    assert_ne!(input.read_line(&mut line).unwrap(), 0);
    let msg: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(msg["method"], "initialize");
    let mut out = io::stdout().lock();
    writeln!(out).unwrap();
    edge_child_send(&mut out, json!({"jsonrpc":"2.0","id":msg["id"]}));
    edge_child_send(
        &mut out,
        json!({"jsonrpc":"2.0","id":msg["id"],"result":{},"error":{"code":-32603}}),
    );
    edge_child_send(
        &mut out,
        json!({"jsonrpc":"2.0","id":msg["id"],"error":{"code":-32603,"message":"fixture refusal"}}),
    );
}

#[test]
fn acp_edge_option_child() {
    if !edge_child_requested("acp_edge_option_child") {
        return;
    }
    let input = BufReader::new(io::stdin().lock());
    let mut out = io::stdout().lock();
    writeln!(out).unwrap();
    for line in input.lines() {
        let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let result = match msg["method"].as_str().unwrap() {
            "initialize" => json!({"agentCapabilities":{}}),
            "session/new" => json!({"sessionId":"s","modes":{
                "currentModeId":"build","availableModes":[{"id":"build","name":"Build"},{"id":"plan","name":"Plan"}]
            }}),
            "session/set_mode" => std::process::exit(7),
            other => panic!("unexpected fixture request {other}"),
        };
        edge_child_send(
            &mut out,
            json!({"jsonrpc":"2.0","id":msg["id"],"result":result}),
        );
        if msg["params"]["cwd"] == "fixture-idle" {
            std::process::exit(8);
        }
    }
}

#[test]
fn malformed_acp_replies_do_not_settle_initialize() {
    let mut node = Node::start_with("acp_edge_noise_child");
    let reply = node.call("initialize", json!({}));
    assert_eq!(reply["error"]["data"]["code"], "adapter.agent_refused");
    let all = notices(&node);
    let refusal = all
        .iter()
        .find(|note| note["code"] == "adapter.agent_refused")
        .unwrap();
    assert_eq!(refusal["params"]["detail"], "fixture refusal");
    node.wait(|m| {
        m["method"] == "log"
            && m["params"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("adapter.agent_exited"))
    });
    assert!(node.done.take().unwrap().join().unwrap().is_err());
}

#[test]
fn an_agent_exit_during_option_change_reports_the_option_stage() {
    let mut node = Node::start_with("acp_edge_option_child");
    assert_eq!(
        node.call("initialize", json!({}))["result"]["adapterName"],
        "acp-bridge"
    );
    assert_eq!(
        node.call("session/start", json!({}))["result"]["sessionId"],
        "s"
    );
    let reply = node.call(
        "session/set_option",
        json!({"kind":"SESSION_OPTION_KIND_MODE","id":"plan"}),
    );
    assert_eq!(reply["error"]["data"]["params"]["stage"], "option");
    assert!(node.done.take().unwrap().join().unwrap().is_err());
    assert!(logs(&node).iter().any(
        |entry| entry["code"] == "adapter.agent_exited" && entry["params"]["stage"] == "option"
    ));
    assert!(notices(&node).iter().any(
        |entry| entry["code"] == "adapter.agent_exited" && entry["params"]["stage"] == "option"
    ));
}

#[test]
fn acp_session_identity_child() {
    if !edge_child_requested("acp_session_identity_child") {
        return;
    }
    let input = BufReader::new(io::stdin().lock());
    let mut out = io::stdout().lock();
    writeln!(out).unwrap();
    let mut held_prompt = None;
    for line in input.lines() {
        let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let sid = &msg["params"]["sessionId"];
        let result = match msg["method"].as_str().unwrap() {
            "initialize" => json!({"agentCapabilities":{"loadSession":true}}),
            "session/load" if sid == "missing" => {
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,
                        "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"partial history"}}}}),
                );
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","id":msg["id"],"error":{"code":-32603,"message":"missing session"}}),
                );
                continue;
            }
            // Review of #1103 (B1): a replay that ends with text, answered at once.
            "session/load" if sid == "text-last" => {
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,
                        "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"last words"}}}}),
                );
                json!({})
            }
            "session/load" if sid == "replayed" => {
                for update in [
                    json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"restored history"}}),
                    json!({"sessionUpdate":"plan","entries":[{"content":"restored plan"}]}),
                ] {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":update}}),
                    );
                }
                json!({})
            }
            "session/load" => json!({}),
            "session/new" if msg["params"]["cwd"] == "missing-id" => json!({}),
            "session/new" if msg["params"]["cwd"] == "turn-exiting" => {
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","id":msg["id"],"result":{"sessionId":"turn-exiting"}}),
                );
                std::process::exit(21);
            }
            "session/new" if msg["params"]["cwd"] == "selection-empty" => {
                json!({"sessionId":"selection-empty"})
            }
            "session/new"
                if matches!(
                    msg["params"]["cwd"].as_str(),
                    Some("selection-first" | "selection-second")
                ) =>
            {
                let session = &msg["params"]["cwd"];
                let mut result = json!({"sessionId":session,
                    "modes":{"availableModes":[{"id":"build","name":"Build"}]},
                    "models":{"availableModels":[{"modelId":"fixture/model","name":"Model"}]}});
                if session == "selection-first" {
                    result["modes"]["currentModeId"] = json!("build");
                    result["models"]["currentModelId"] = json!("fixture/model");
                }
                result
            }
            "session/new" => json!({"sessionId":"created"}),
            "session/prompt" if msg["params"]["prompt"][0]["text"] == "hold-turn" => {
                held_prompt = Some(msg["id"].clone());
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,
                        "update":{"sessionUpdate":"plan","entries":[{"content":"turn held"}]}}}),
                );
                continue;
            }
            "session/prompt" => {
                if let Some(id) = held_prompt.take() {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","id":id,"result":{"stopReason":"refusal"}}),
                    );
                }
                if msg["params"]["prompt"][0]["text"] == "late-usage" {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"usage-first",
                            "update":{"sessionUpdate":"usage_update","cost":{"currency":"USD","amount":0.09},
                                "used":900,"size":1000}}}),
                    );
                }
                if let Some(amount) = match sid.as_str() {
                    Some("usage-first") => Some(0.04),
                    Some("usage-second") => Some(0.01),
                    _ => None,
                } {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,
                            "update":{"sessionUpdate":"usage_update","cost":{"currency":"USD","amount":amount},
                                "used":12,"size":100}}}),
                    );
                }
                edge_child_send(
                    &mut out,
                    json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,
                        "update":{"sessionUpdate":"plan","entries":[{"content":sid}]}}}),
                );
                json!({"stopReason":"end_turn"})
            }
            other => panic!("unexpected fixture request {other}"),
        };
        edge_child_send(
            &mut out,
            json!({"jsonrpc":"2.0","id":msg["id"],"result":result}),
        );
    }
}

#[test]
fn only_a_load_can_use_the_session_id_from_its_request() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    let loaded = node.call("session/start", json!({"resumeSessionId":"previous"}));
    assert_eq!(loaded["result"]["sessionId"], "previous");

    let missing = node.call("session/start", json!({"worktree":"missing-id"}));
    assert_eq!(
        missing["error"]["data"],
        json!({"code":"adapter.agent_refused","params":{"stage":"session"}})
    );

    let created = node.call("session/start", json!({}));
    assert_eq!(created["result"]["sessionId"], "created");
}

#[test]
fn the_next_session_reports_its_own_options_even_when_empty() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    for (sid, mode, model, count) in [
        ("selection-first", "build", "fixture/model", 1),
        ("selection-second", "", "", 1),
        ("selection-empty", "", "", 0),
    ] {
        let reply = node.call("session/start", json!({"worktree":sid}));
        assert_eq!(reply["result"]["sessionId"], sid);
        let message =
            node.wait(|m| m["method"] == "session/options" && m["params"]["sessionId"] == sid);
        let params = &message["params"];
        assert_eq!(params["currentMode"], mode);
        assert_eq!(params["currentModel"], model);
        assert_eq!(params["modes"].as_array().unwrap().len(), count);
        assert_eq!(params["models"].as_array().unwrap().len(), count);
    }
}

#[test]
fn usage_baselines_are_independent_between_sessions() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    for sid in ["usage-first", "usage-second", "usage-first"] {
        let response = node.call("session/start", json!({"resumeSessionId":sid}));
        assert_eq!(response["result"]["sessionId"], sid);
        assert_eq!(turn_end(&mut node, "usage"), "");
    }

    let reports: Vec<_> = node
        .messages()
        .into_iter()
        .filter(|message| message["method"] == "usage/report")
        .map(|message| message["params"].clone())
        .collect();
    assert_eq!(reports.len(), 2, "{reports:#?}");
    for (report, (sid, cost)) in reports
        .iter()
        .zip([("usage-first", 40_000), ("usage-second", 10_000)])
    {
        assert_eq!(report["sessionId"], sid);
        assert_eq!(report["costMicros"], cost);
        assert_eq!(report["contextTokens"], 12);
        assert_eq!(report["contextWindow"], 100);
    }
}

#[test]
fn late_usage_from_the_previous_session_does_not_change_the_active_session() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    for sid in ["usage-first", "usage-second"] {
        let reply = node.call("session/start", json!({"resumeSessionId":sid}));
        assert_eq!(reply["result"]["sessionId"], sid);
    }
    assert_eq!(turn_end(&mut node, "late-usage"), "");

    let reports: Vec<_> = node
        .messages()
        .into_iter()
        .filter(|message| message["method"] == "usage/report")
        .map(|message| message["params"].clone())
        .collect();
    assert_eq!(reports.len(), 1, "{reports:#?}");
    assert_eq!(reports[0]["sessionId"], "usage-second");
    assert_eq!(reports[0]["costMicros"], 10_000);
    assert_eq!(reports[0]["contextTokens"], 12);
}

fn node_with_held_prompt() -> Node {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    let reply = node.call("session/start", json!({"resumeSessionId":"turn-held"}));
    assert_eq!(reply["result"]["sessionId"], "turn-held");
    assert_eq!(
        node.call("session/prompt", json!({"text":"hold-turn"}))["result"],
        json!({})
    );
    node.wait(|m| m["params"]["plan"]["steps"][0] == "turn held");
    node
}

#[test]
fn a_replaced_sessions_prompt_does_not_finish_the_new_turn() {
    let mut node = node_with_held_prompt();
    let reply = node.call("session/start", json!({"resumeSessionId":"turn-current"}));
    assert_eq!(reply["result"]["sessionId"], "turn-current");
    assert_eq!(turn_end(&mut node, "complete-turn"), "");
    assert!(notices(&node)
        .iter()
        .all(|notice| notice["code"] != "adapter.turn_stopped"));
}

#[test]
fn a_replaced_sessions_prompt_does_not_mark_an_idle_exit_as_a_turn() {
    let mut node = node_with_held_prompt();
    let reply = node.call("session/start", json!({"worktree":"turn-exiting"}));
    assert_eq!(reply["result"]["sessionId"], "turn-exiting");
    assert!(node.done.take().unwrap().join().unwrap().is_err());
    let log = logs(&node)
        .into_iter()
        .find(|log| log["code"] == "adapter.agent_exited")
        .unwrap();
    assert_eq!(log["params"]["stage"], "idle");
    assert_eq!(log["params"]["exit_code"], "21");
}

/// Review of #1103 (B1): what an agent replays when it loads a session reaches the node before the answer to `session/start`, text
/// it was still holding for its 150 ms window included: the node takes everything before that answer as the replay.
#[test]
fn a_loaded_sessions_replay_comes_before_its_answer() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    let loaded = node.call("session/start", json!({"resumeSessionId":"text-last"}));
    assert_eq!(loaded["result"]["sessionId"], "text-last");
    let messages = node.messages();
    let answer = messages
        .iter()
        .position(|m| m.get("id") == loaded.get("id") && m.get("result").is_some())
        .expect("the answer");
    let history = messages
        .iter()
        .position(|m| m["params"]["text"]["text"] == "last words")
        .expect("the replayed text, before the answer");
    assert!(history < answer, "{messages:#?}");
}

#[test]
fn a_refused_load_keeps_the_active_session() {
    let mut node = Node::start_with("acp_session_identity_child");
    node.call("initialize", json!({}));
    let loaded = node.call("session/start", json!({"resumeSessionId":"replayed"}));
    assert_eq!(loaded["result"]["sessionId"], "replayed");
    let history = node.wait(|m| m["params"]["text"]["text"] == "restored history");
    assert_eq!(history["params"]["sessionId"], "replayed");

    let refused = node.call("session/start", json!({"resumeSessionId":"missing"}));
    assert_eq!(refused["error"]["data"]["code"], "adapter.agent_refused");
    assert_eq!(
        node.call("session/prompt", json!({"text":"next"}))["result"],
        json!({})
    );
    let plan = node.wait(|m| {
        m["method"] == "chat/append"
            && m["params"]["plan"]["steps"][0]
                .as_str()
                .is_some_and(|s| s != "restored plan")
    });
    assert_eq!(plan["params"]["plan"]["steps"][0], "replayed");
    let partial = node.wait(|m| m["params"]["text"]["text"] == "partial history");
    assert_eq!(partial["params"]["sessionId"], "missing");
}

#[test]
fn acp_exit_after_initialize_child() {
    if !edge_child_requested("acp_exit_after_initialize_child") {
        return;
    }
    let request = BufReader::new(io::stdin().lock())
        .lines()
        .next()
        .unwrap()
        .unwrap();
    let init: Value = serde_json::from_str(&request).unwrap();
    assert_eq!(init["method"], "initialize");

    let mut out = io::stdout().lock();
    writeln!(out).unwrap();
    edge_child_send(
        &mut out,
        json!({"jsonrpc":"2.0","id":init["id"],"result":{"agentCapabilities":{}}}),
    );
    std::process::exit(11);
}

#[test]
fn exit_after_initialize_is_reported_at_the_session_stage() {
    let mut node = Node::start_with("acp_exit_after_initialize_child");
    assert_eq!(
        node.call("initialize", json!({}))["result"]["adapterName"],
        "acp-bridge"
    );
    node.wait(|message| message["method"] == "log");
    let log = logs(&node)
        .into_iter()
        .find(|log| log["code"] == "adapter.agent_exited")
        .unwrap();
    let notice = notices(&node)
        .into_iter()
        .find(|notice| notice["code"] == "adapter.agent_exited")
        .unwrap();
    let reply = node.call("session/start", json!({}));
    let error = &reply["error"]["data"];
    for diagnostic in [&log, &notice, error] {
        assert_eq!(diagnostic["code"], "adapter.agent_exited");
        assert_eq!(diagnostic["params"]["stage"], "session");
        assert_eq!(diagnostic["params"]["exit_code"], "11");
    }
    assert!(node.done.take().unwrap().join().unwrap().is_err());
}

#[test]
fn a_late_response_does_not_consume_the_exit_diagnostic() {
    let mut node = Node::start_with("acp_exit_after_initialize_child");
    node.call("initialize", json!({}));
    node.wait(|message| message["method"] == "log");
    // The response and the following call share the same input stream.
    let unrelated = json!({"jsonrpc":"2.0","id":90,"result":{}});
    node.to_bridge
        .send(format!("{unrelated}\n").into_bytes())
        .unwrap();
    let reply = node.call("session/start", json!({}));
    assert_eq!(reply["error"]["data"]["code"], "adapter.agent_exited");
    assert!(node.done.take().unwrap().join().unwrap().is_err());
    assert!(!node.messages().iter().any(|msg| msg["id"] == 90));
}

#[test]
fn acp_cancel_pending_child() {
    if !edge_child_requested("acp_cancel_pending_child") {
        return;
    }
    let input = BufReader::new(io::stdin().lock());
    let mut out = io::stdout().lock();
    writeln!(out).unwrap();
    let mut prompt = None;
    let mut pending = std::collections::BTreeSet::new();
    let mut cancelled = false;
    let mut late = false;
    for line in input.lines() {
        let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let result = match message["method"].as_str() {
            Some("initialize") => json!({"agentCapabilities":{}}),
            Some("session/new") => json!({"sessionId":"cancel-fixture"}),
            Some("session/prompt") => {
                prompt = Some(message["id"].clone());
                late = message["params"]["prompt"][0]["text"] == "late";
                if late {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cancel-fixture",
                            "update":{"sessionUpdate":"plan","entries":[{"content":"prompt held"}]}}}),
                    );
                } else {
                    send_cancel_fixture_confirmations(&mut out, &mut pending);
                }
                continue;
            }
            Some("session/cancel") => {
                assert_eq!(message["params"]["sessionId"], "cancel-fixture");
                cancelled = true;
                if std::mem::take(&mut late) {
                    send_cancel_fixture_confirmations(&mut out, &mut pending);
                }
                continue;
            }
            None => {
                assert!(cancelled);
                assert!(pending.remove(&message["id"].as_u64().unwrap()));
                assert_eq!(message["result"]["outcome"], json!({"outcome":"cancelled"}));
                if pending.is_empty() {
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"cancel-fixture",
                            "update":{"sessionUpdate":"plan","entries":[{"content":"confirmations cleared"}]}}}),
                    );
                    edge_child_send(
                        &mut out,
                        json!({"jsonrpc":"2.0","id":prompt.take().unwrap(),"result":{"stopReason":"cancelled"}}),
                    );
                }
                continue;
            }
            other => panic!("unexpected fixture method {other:?}"),
        };
        edge_child_send(
            &mut out,
            json!({"jsonrpc":"2.0","id":message["id"],"result":result}),
        );
    }
}

fn send_cancel_fixture_confirmations(
    out: &mut impl Write,
    pending: &mut std::collections::BTreeSet<u64>,
) {
    for (id, title) in [
        (90, "First fixture confirmation"),
        (91, "Second fixture confirmation"),
    ] {
        assert!(pending.insert(id));
        edge_child_send(
            out,
            json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{
                "sessionId":"cancel-fixture","toolCall":{"title":title},"options":[{"optionId":"yes","kind":"allow_once"}]
            }}),
        );
    }
}

#[test]
fn cancelling_a_prompt_answers_every_pending_confirmation() {
    let mut node = Node::start_with("acp_cancel_pending_child");
    node.call("initialize", json!({}));
    assert_eq!(
        node.call("session/start", json!({}))["result"]["sessionId"],
        "cancel-fixture"
    );
    node.call("session/prompt", json!({"text":"fixture"}));
    node.wait(|message| {
        message["method"] == "approval/request"
            && message["params"]["action"] == "Second fixture confirmation"
    });
    assert_eq!(node.call("session/cancel", json!({}))["result"], json!({}));
    node.wait(|message| message["params"]["plan"]["steps"][0] == "confirmations cleared");
    assert_eq!(node.call("shutdown", json!({}))["result"], json!({}));
    node.done.take().unwrap().join().unwrap().unwrap();
}

#[test]
fn a_confirmation_arriving_during_cancellation_is_cancelled() {
    let mut node = Node::start_with("acp_cancel_pending_child");
    node.call("initialize", json!({}));
    assert_eq!(
        node.call("session/start", json!({}))["result"]["sessionId"],
        "cancel-fixture"
    );
    node.call("session/prompt", json!({"text":"late"}));
    node.wait(|message| message["params"]["plan"]["steps"][0] == "prompt held");
    assert_eq!(node.call("session/cancel", json!({}))["result"], json!({}));
    node.wait(|message| message["params"]["plan"]["steps"][0] == "confirmations cleared");
    assert!(node
        .messages()
        .iter()
        .all(|message| message["method"] != "approval/request"));
    assert!(!node.state("AGENT_STATE_BLOCKED"));
    assert_eq!(node.call("shutdown", json!({}))["result"], json!({}));
    node.done.take().unwrap().join().unwrap().unwrap();
}

/// ADR-0090: the fake agent that offers config options and slash commands.
#[test]
fn acp_child_config() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_config")
    {
        return;
    }
    println!();
    std::env::set_var("AGORA_FAKE_ACP_CONFIG", "1");
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// A bridge whose node said it takes rich chat (ADR-0090), with its session open.
fn started_rich(child: &str) -> Node {
    let mut n = Node::start_with(child);
    let init = n.call(
        "initialize",
        json!({"protocolGeneration": 1, "nodeCapabilities": ["chat", "chat.rich"]}),
    );
    let caps: Vec<&str> = init["result"]["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for c in [
        "thought",
        "tool_status",
        "plan_status",
        "config",
        "commands",
        "turns",
    ] {
        assert!(caps.contains(&c), "{c} missing: {caps:?}");
    }
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    n.wait(|m| m["method"] == "state/update" && m["params"]["state"] == "AGENT_STATE_IDLE");
    n
}

fn tool_items(n: &Node) -> Vec<Value> {
    chats(n)
        .into_iter()
        .filter_map(|p| p.get("toolCall").cloned())
        .collect()
}

/// ADR-0090: thinking, a tool call's whole life (matched by its id), and a plan with step status reach the node as items.
#[test]
fn a_rich_node_gets_thinking_tool_status_and_plan_steps() {
    let mut n = started_rich("acp_child");
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "rich"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "done");
    let c = chats(&n);
    // The two chunks are joined into one thought, which comes before the tool call.
    let thought = c
        .iter()
        .position(|p| p.get("thought").is_some())
        .expect("a thought");
    assert_eq!(c[thought]["thought"]["text"], "thinking hard");
    let first_tool = c
        .iter()
        .position(|p| p.get("toolCall").is_some())
        .expect("a tool call");
    assert!(thought < first_tool);
    // One call id, three versions: the first carries everything the call began with, the rest only what changed.
    let calls = tool_items(&n);
    assert_eq!(calls.len(), 3, "{calls:#?}");
    assert!(calls.iter().all(|t| t["callId"] == "t3"));
    assert_eq!(calls[0]["tool"], "read a.rs");
    assert_eq!(calls[0]["kind"], "read");
    assert_eq!(calls[0]["status"], "pending");
    assert_eq!(calls[0]["locations"], json!([{"path": "a.rs", "line": 3}]));
    assert_eq!(calls[0]["argumentsJson"], "{\"path\":\"a.rs\"}");
    assert_eq!(calls[1]["status"], "in_progress");
    assert!(calls[1].get("tool").is_none() && calls[1].get("output").is_none());
    assert_eq!(calls[2]["status"], "completed");
    assert_eq!(calls[2]["output"], "file text");
    // The plan keeps its old lines and adds the entries with status and priority; the later plan has the new status.
    let plans: Vec<&Value> = c.iter().filter_map(|p| p.get("plan")).collect();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0]["steps"], json!(["read the code", "fix the bug"]));
    assert_eq!(plans[0]["entries"][0]["status"], "in_progress");
    assert_eq!(plans[1]["entries"][0]["status"], "completed");
    assert_eq!(plans[1]["entries"][1]["priority"], "medium");
}

/// ADR-0090: a node that did not say `chat.rich` is served exactly as before: no thought, no call id or status, plan lines only.
#[test]
fn a_node_without_rich_chat_gets_the_old_items_byte_for_byte() {
    let mut n = Node::start();
    let init = n.call(
        "initialize",
        json!({"protocolGeneration": 1, "nodeCapabilities": ["chat"]}),
    );
    assert_eq!(
        init["result"]["capabilities"],
        json!([
            "chat",
            "approvals",
            "approval_scopes",
            "plan",
            "diff",
            "cancel",
            "usage",
            "set_mode",
            "set_model"
        ])
    );
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "rich"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "done");
    let c = chats(&n);
    assert!(c.iter().all(|p| p.get("thought").is_none()), "{c:#?}");
    let calls = tool_items(&n);
    assert_eq!(
        calls,
        [json!({"tool": "read a.rs", "argumentsJson": "{\"path\":\"a.rs\"}"})]
    );
    let plans: Vec<&Value> = c.iter().filter_map(|p| p.get("plan")).collect();
    assert!(plans
        .iter()
        .all(|p| p.get("entries").is_none() && p["steps"].is_array()));
    assert!(n
        .messages()
        .iter()
        .all(|m| m["method"] != "session/config" && m["method"] != "session/commands"));
}

/// ADR-0090: a tool's output is cut (head and tail kept) and anything that looks like a secret is replaced before the node sees it.
#[test]
fn a_tool_output_is_bounded_and_its_secrets_are_removed() {
    let mut n = started_rich("acp_child");
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "rich-secret"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "done");
    let out = tool_items(&n)
        .into_iter()
        .find_map(|t| t.get("output").and_then(Value::as_str).map(str::to_string))
        .expect("an output");
    assert!(out.len() <= 64 * 1024, "{}", out.len());
    assert!(
        !out.contains("sk-fake0123456789abcdefghijklmnop"),
        "a secret leaked"
    );
    assert!(out.contains("[redacted]"));
}

/// ADR-0090: config options and commands are reported, a value is set with the ACP call, and a bad id or value never reaches the
/// agent. A mode value no list of bad names has is flagged; a boolean about approvals is; an unrelated one is not.
#[test]
fn config_options_and_commands_are_reported_and_can_be_set() {
    let mut n = started_rich("acp_child_config");
    let cfg = n.wait(|m| m["method"] == "session/config");
    let options = cfg["params"]["options"].as_array().unwrap();
    let by = |id: &str| {
        options
            .iter()
            .find(|o| o["id"] == id)
            .unwrap_or_else(|| panic!("{options:#?}"))
    };
    assert_eq!(by("effort")["category"], "thought_level");
    assert_eq!(by("effort")["current"], "medium");
    assert_eq!(by("effort")["choices"].as_array().unwrap().len(), 3);
    assert!(by("effort")["choices"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c.get("skipsPermissions").is_none()));
    let choice = |c: &str| {
        by("approvals")["choices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == c)
            .unwrap()
            .clone()
    };
    assert!(
        choice("turbo")["skipsPermissions"] == true,
        "an unlisted name is not safe"
    );
    assert!(choice("default").get("skipsPermissions").is_none());
    assert!(choice("plan").get("skipsPermissions").is_none());
    assert_eq!(by("auto_approve")["type"], "boolean");
    assert_eq!(by("auto_approve")["skipsPermissions"], true);
    assert!(by("extended").get("skipsPermissions").is_none());
    let cmds = n.wait(|m| m["method"] == "session/commands");
    assert_eq!(
        cmds["params"]["commands"][0],
        json!({"name": "review", "description": "Review the changes", "inputHint": "a path"})
    );
    assert_eq!(cmds["params"]["commands"][1]["inputHint"], "");

    // Setting a select: the agent takes it and the node hears the new current value.
    let set = n.call(
        "session/set_config",
        json!({"sessionId": "acp-s1", "configId": "effort", "value": "high"}),
    );
    assert_eq!(set["result"], json!({}), "{set}");
    n.wait(|m| {
        m["method"] == "session/config"
            && m["params"]["options"].as_array().is_some_and(|o| {
                o.iter()
                    .any(|o| o["id"] == "effort" && o["current"] == "high")
            })
    });
    // A boolean goes to the agent as a boolean.
    assert_eq!(
        n.call(
            "session/set_config",
            json!({"configId": "extended", "value": "true"})
        )["result"],
        json!({})
    );
    // Not what the agent reported: refused here, with the stable code.
    for (id, value) in [
        ("effort", "extreme"),
        ("nope", "x"),
        ("extended", "yes"),
        ("effort", ""),
    ] {
        let r = n.call(
            "session/set_config",
            json!({"configId": id, "value": value}),
        );
        assert_eq!(
            r["error"]["data"]["code"], "adapter.option_refused",
            "{id} {value}: {r}"
        );
    }
    // The agent sent its own `modes`, so the lists are not built from its options.
    let opts = n
        .messages()
        .into_iter()
        .rev()
        .find(|m| m["method"] == "session/options")
        .unwrap();
    assert_eq!(opts["params"]["modes"].as_array().unwrap().len(), 2);
}

/// ADR-0090: `session/cancel` ends the turn and leaves the agent alive and usable; only a stop ends it.
#[test]
fn cancelling_a_turn_leaves_the_agent_alive() {
    let mut n = started_rich("acp_child");
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "slow"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["toolCall"]["callId"] == "t4");
    assert!(n.state("AGENT_STATE_WORKING"));
    let before = n.messages().len();
    n.call("session/cancel", json!({"sessionId": "acp-s1"}));
    // The turn ended: idle, with no notice (a cancelled turn is a normal return to idle) and the bridge still running.
    let end = Instant::now() + Duration::from_secs(10);
    while !n.messages()[before..]
        .iter()
        .any(|m| m["params"]["state"] == "AGENT_STATE_IDLE")
    {
        assert!(Instant::now() < end, "{:#?}", n.messages());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(chats(&n).iter().all(|p| p.get("notice").is_none()));
    assert!(n.done.as_ref().is_some_and(|d| !d.is_finished()));
    n.call(
        "session/prompt",
        json!({"sessionId": "acp-s1", "text": "again"}),
    );
    n.wait(|m| m["method"] == "chat/append" && m["params"]["text"]["text"] == "again");
    // A stop ends it.
    n.call(
        "session/stop",
        json!({"sessionId": "acp-s1", "reason": "user"}),
    );
    assert!(n.done.take().unwrap().join().unwrap().is_ok());
}

/// ADR-0090: the fake agent that offers config options and no mode or model list.
#[test]
fn acp_child_config_only() {
    if std::env::var_os("AGORA_TEST_ACP").is_none()
        || !std::env::args().any(|a| a == "acp_child_config_only")
    {
        return;
    }
    println!();
    std::env::set_var("AGORA_FAKE_ACP_CONFIG", "1");
    std::env::set_var("AGORA_FAKE_ACP_CONFIG_ONLY", "1");
    let _ = agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock());
}

/// ADR-0090: an agent that gives config options and no mode list still gets a mode switch, built from its `mode` option and set
/// the ACP way; the values the rule cannot place stay flagged.
#[test]
fn a_mode_list_is_built_from_the_config_options_when_the_agent_gave_none() {
    let mut n = started_rich("acp_child_config_only");
    let opts = n.wait(|m| {
        m["method"] == "session/options"
            && m["params"]["modes"]
                .as_array()
                .is_some_and(|m| !m.is_empty())
    });
    let modes = opts["params"]["modes"].as_array().unwrap();
    assert_eq!(
        modes
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["default", "turbo", "plan"]
    );
    assert_eq!(opts["params"]["currentMode"], "default");
    assert!(modes[1]["skipsPermissions"] == true);
    let set = n.call(
        "session/set_option",
        json!({"kind": "SESSION_OPTION_KIND_MODE", "id": "plan"}),
    );
    assert_eq!(set["result"], json!({}), "{set}");
    n.wait(|m| m["method"] == "session/options" && m["params"]["currentMode"] == "plan");
}

/// ADR-0090: the same agent against a node without rich chat: nothing is built or reported, as before.
#[test]
fn a_node_without_rich_chat_sees_no_config_options() {
    let mut n = Node::start_with("acp_child_config_only");
    n.call("initialize", json!({"nodeCapabilities": ["chat"]}));
    n.call(
        "session/start",
        json!({"projectRoot": ".", "worktree": "."}),
    );
    n.wait(|m| m["method"] == "state/update" && m["params"]["state"] == "AGENT_STATE_IDLE");
    assert!(n.messages().iter().all(|m| m["method"] != "session/config"));
    let opts = n
        .messages()
        .into_iter()
        .find(|m| m["method"] == "session/options")
        .unwrap();
    assert_eq!(opts["params"]["modes"], json!([]));
    let refused = n.call(
        "session/set_config",
        json!({"configId": "effort", "value": "high"}),
    );
    assert_eq!(refused["error"]["data"]["code"], "adapter.option_refused");
}
