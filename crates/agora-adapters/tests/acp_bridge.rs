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
        if let Some(m) = msgs[before..].iter().find(|m| idle(m)) {
            return m["params"]["message"].as_str().unwrap_or("").to_string();
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
    std::env::set_var("AGORA_FAKE_ACP_BYPASS_MODE", "1");
    std::env::set_var("AGORA_FAKE_ACP_REFUSE_MODEL", "1");
    let mut n = Node::start();
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
