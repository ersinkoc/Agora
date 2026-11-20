//! An ACP (L2) agent through a real node: the manifest says `[structured] acp`, the node runs `agora acp-bridge` around the agent,
//! and everything else (start, prompt, transcript) works as for any chat agent. The agent is the fake ACP agent, never a real one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use agora_node::control::{Client, KIND_CHAT_ENTRY, KIND_CHAT_SNAPSHOT};
use agora_protocol::v1::{self, command_request::Command as Cmd};
use prost::Message;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn fake_agent() -> String {
    let mut p = std::path::PathBuf::from(AGORA);
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
    p.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

struct Node(std::process::Child);
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn agora(home: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(AGORA)
        .args(args)
        .current_dir(home.join("demo"))
        .env("AGORA_HOME", home)
        .env("AGORA_LANG", "en")
        .output()
        .expect("run agora");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    )
}

#[test]
fn an_acp_agent_runs_through_the_bridge_like_any_chat_agent() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let home = std::env::temp_dir().join(format!("agora-acp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    std::fs::write(
        home.join("manifests/acp-fake.toml"),
        format!("kind = \"acp-fake\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"] }}\n", fake_agent()),
    )
    .unwrap();
    let node = Node(
        Command::new(AGORA)
            .args(["node", "run", "--project"])
            .arg(&repo)
            .env("AGORA_HOME", &home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start node"),
    );
    let sock = home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }

    let (code, kinds) = agora(&home, &["agent", "kinds"]);
    assert!(code == 0 && kinds.contains("acp-fake"), "{kinds}");
    let (code, started) = agora(&home, &["agent", "start", "acp-fake"]);
    assert_eq!(code, 0, "{started}");
    let id = started
        .split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string();
    wait_ready(&home, &id);

    let mut c = Client::connect(&sock).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: id.clone(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(&home, &["agent", "prompt", &id, "plan"]);
    assert_eq!(code, 0, "{out}");

    let end = Instant::now() + Duration::from_secs(15);
    let mut kinds_seen = Vec::new();
    let mut texts = Vec::new();
    while Instant::now() < end && !texts.iter().any(|t: &String| t == "hello world") {
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 {
            continue;
        }
        let entries = match f.kind {
            KIND_CHAT_SNAPSHOT => {
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries
            }
            KIND_CHAT_ENTRY => vec![v1::ChatEntry::decode(f.payload.as_slice()).unwrap()],
            _ => continue,
        };
        for e in entries {
            kinds_seen.push(e.kind);
            texts.push(e.text);
        }
    }
    assert!(
        texts.iter().any(|t| t == "hello world"),
        "the agent's message arrived joined: {texts:?}"
    );
    assert!(
        kinds_seen.contains(&(v1::ChatKind::Plan as i32))
            && kinds_seen.contains(&(v1::ChatKind::Diff as i32))
            && kinds_seen.contains(&(v1::ChatKind::ToolCall as i32)),
        "{kinds_seen:?}"
    );
    // The state the bridge reports (idle once the turn ended) reaches `agent list`; it used to stay `unknown`.
    let end = Instant::now() + Duration::from_secs(15);
    let listed = loop {
        let (_, out) = agora(&home, &["agent", "list"]);
        if out.contains("idle") || Instant::now() > end {
            break out;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(listed.contains("idle"), "{listed}");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A node with two ACP kinds: `acp-fake` (the fake agent) and `acp-broken` (one that dies before `initialize`).
fn node_with_acp(tag: &str) -> Option<(std::path::PathBuf, Node)> {
    node_with_acp_policy(tag, None)
}

/// Like [`node_with_acp`]; with `policy` the node's policy file holds it (the kind `acp-bypass` offers a mode that skips permissions).
fn node_with_acp_policy(tag: &str, policy: Option<&str>) -> Option<(std::path::PathBuf, Node)> {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return None;
    }
    let home = std::env::temp_dir().join(format!("agora-acp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    for (kind, extra) in [
        ("acp-fake", ""),
        ("acp-broken", ", \"--fail-at-start\""),
        ("acp-bypass", ", \"--bypass-mode\""),
        ("acp-config", ", \"--config-options\""),
    ] {
        std::fs::write(
            home.join(format!("manifests/{kind}.toml")),
            format!("kind = \"{kind}\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"{extra}] }}\n", fake_agent()),
        )
        .unwrap();
    }
    if let Some(policy) = policy {
        std::fs::create_dir_all(home.join("data")).unwrap();
        std::fs::write(home.join("data/policy"), policy).unwrap();
    }
    let mut cmd = Command::new(AGORA);
    cmd.args(["node", "run", "--project"])
        .arg(&repo)
        .env("AGORA_HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let node = Node(cmd.spawn().expect("start node"));
    let sock = home.join("node.sock");
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
    Some((home, node))
}

fn started_agent(home: &Path) -> String {
    started_agent_of(home, "acp-fake")
}

fn started_agent_of(home: &Path, kind: &str) -> String {
    let (code, started) = agora(home, &["agent", "start", kind]);
    assert_eq!(code, 0, "{started}");
    let id = started
        .split_whitespace()
        .find(|w| w.starts_with("agent-"))
        .expect("an agent id")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
        .to_string();
    wait_ready(home, &id);
    id
}

/// Waits until the bridge has its session (the agent is `idle`): `agent start` answers once the process runs, and a prompt sent
/// before the session exists is refused ("no session"), so a slow runner (seen on Windows CI) lost the prompt and its answer.
fn wait_ready(home: &Path, id: &str) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (_, out) = agora(home, &["agent", "list"]);
        if out.lines().any(|l| l.contains(id) && l.contains("idle")) {
            return;
        }
        assert!(Instant::now() < end, "{id} never became idle: {out}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ADR-0058: a turn that ends in silence is a notice in the conversation, with a code and no text.
#[test]
fn a_silent_turn_reaches_the_conversation_as_a_notice() {
    let Some((home, node)) = node_with_acp("notice") else {
        return;
    };
    let id = started_agent(&home);
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: id.clone(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(&home, &["agent", "prompt", &id, "silent"]);
    assert_eq!(code, 0, "{out}");
    let end = Instant::now() + Duration::from_secs(15);
    let notice = loop {
        assert!(Instant::now() < end, "no notice arrived");
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 {
            continue;
        }
        let entries = match f.kind {
            KIND_CHAT_SNAPSHOT => {
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries
            }
            KIND_CHAT_ENTRY => vec![v1::ChatEntry::decode(f.payload.as_slice()).unwrap()],
            _ => continue,
        };
        if let Some(n) = entries
            .into_iter()
            .find(|e| e.kind == v1::ChatKind::Notice as i32)
        {
            break n;
        }
    };
    assert_eq!(notice.code, "adapter.turn_empty");
    assert!(notice.text.is_empty());
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// ADR-0058: the user switches the agent's mode and model among what it offered; anything else is refused before it reaches the
/// agent.
#[test]
fn the_user_switches_an_acp_agents_mode_and_model() {
    let Some((home, node)) = node_with_acp("modes") else {
        return;
    };
    let id = started_agent(&home);
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    let set_mode = |c: &mut Client, mode: &str| {
        c.call(Cmd::AgentSetMode(v1::AgentSetMode {
            agent_id: id.clone(),
            mode: mode.into(),
        }))
        .unwrap()
    };
    // The options arrive just after the start; until then nothing is offered.
    let end = Instant::now() + Duration::from_secs(15);
    while set_mode(&mut c, "plan").is_err() {
        assert!(Instant::now() < end, "the mode was never accepted");
        std::thread::sleep(Duration::from_millis(100));
    }
    let refused = set_mode(&mut c, "nope").unwrap_err();
    assert_eq!(refused.code, "protocol.invalid_message");
    let model = |c: &mut Client, model: &str| {
        c.call(Cmd::AgentSetModel(v1::AgentSetModel {
            agent_id: id.clone(),
            model: model.into(),
        }))
        .unwrap()
    };
    assert!(model(&mut c, "fake/model-2").is_ok());
    assert_eq!(
        model(&mut c, "fake/model-9").unwrap_err().code,
        "protocol.invalid_message"
    );
    let unknown = c
        .call(Cmd::AgentSetModel(v1::AgentSetModel {
            agent_id: "agent-99".into(),
            model: "fake/model-2".into(),
        }))
        .unwrap()
        .unwrap_err();
    assert_eq!(unknown.code, "agent.not_found");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A mode in which the agent acts without asking follows the node's `skip_permissions` policy, as a launch option does (ADR-0059):
/// `forbid` refuses it with `agent.option_forbidden`, `allow` asks the owner (`agent.approval_needed`); a mode that asks stays possible under `forbid`.
#[test]
fn a_mode_that_skips_permissions_follows_the_nodes_policy() {
    for (policy, allowed) in [
        ("skip_permissions = forbid\n", false),
        ("skip_permissions = allow\n", true),
    ] {
        let tag = if allowed { "skip-allow" } else { "skip-forbid" };
        let Some((home, node)) = node_with_acp_policy(tag, Some(policy)) else {
            return;
        };
        let id = started_agent_of(&home, "acp-bypass");
        let mut c = Client::connect(&home.join("node.sock")).unwrap();
        let mut set_mode = |mode: &str| {
            c.call(Cmd::AgentSetMode(v1::AgentSetMode {
                agent_id: id.clone(),
                mode: mode.into(),
            }))
            .unwrap()
        };
        // The options arrive just after the start.
        let end = Instant::now() + Duration::from_secs(15);
        while set_mode("plan").is_err() {
            assert!(Instant::now() < end, "the mode was never accepted");
            std::thread::sleep(Duration::from_millis(100));
        }
        let skipped = set_mode("bypassPermissions");
        if allowed {
            // The policy allows it: the node neither applies nor refuses it, it asks the owner (ADR-0090 Amendment A).
            let asked = skipped.unwrap_err();
            assert_eq!(asked.code, "agent.approval_needed");
            assert!(param(&asked, "approval_id").is_some());
            assert!(set_mode("build").is_ok());
        } else {
            let err = skipped.unwrap_err();
            assert_eq!(err.code, "agent.option_forbidden");
            assert!(set_mode("build").is_ok());
        }
        drop(node);
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// ADR-0058: an agent that dies while starting is reported with its reason, not as `internal`.
#[test]
fn an_acp_agent_that_dies_while_starting_says_why() {
    let Some((home, node)) = node_with_acp("dies") else {
        return;
    };
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    let projects = match c
        .call(Cmd::ProjectList(v1::ProjectList {}))
        .unwrap()
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::ProjectList(p)) => p.projects,
        other => panic!("{other:?}"),
    };
    let err = c
        .call(Cmd::AgentStart(v1::AgentStart {
            project_id: projects[0].project_id.clone(),
            kind: "acp-broken".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap_err();
    assert_eq!(err.code, "adapter.agent_exited", "{err:?}");
    let param = |k: &str| match err.params.get(k).and_then(|p| p.value.clone()) {
        Some(v1::param_value::Value::Str(s)) => s,
        other => panic!("{k}: {other:?}"),
    };
    assert_eq!(param("stage"), "initialize");
    assert_eq!(param("exit_code"), "5");
    let tail = param("stderr_tail");
    assert!(tail.contains("plugin failed"), "{tail}");
    assert!(!tail.contains("sk-fake"), "a secret leaked: {tail}");
    // The command line shows it in words, from the locale files.
    let (code, out) = agora(&home, &["agent", "start", "acp-broken"]);
    assert_ne!(code, 0);
    assert!(out.contains("exited before it was ready"), "{out}");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// Prompts `agent` and returns the first thing it says (after the prompt) that `want` accepts.
fn answer(home: &Path, agent: &str, prompt: &str, want: impl Fn(&str) -> bool) -> String {
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    c.call_on(
        3,
        Cmd::ChatSubscribe(v1::ChatSubscribe {
            agent_id: agent.into(),
        }),
    )
    .unwrap()
    .expect("subscribed");
    let (code, out) = agora(home, &["agent", "prompt", agent, prompt]);
    assert_eq!(code, 0, "{out}");
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(Instant::now() < end, "{agent} did not answer {prompt}");
        let Some(f) = c.read_frame(Some(Duration::from_millis(200))).unwrap() else {
            continue;
        };
        if f.stream_id != 3 || f.kind != KIND_CHAT_ENTRY {
            continue;
        }
        let e = v1::ChatEntry::decode(f.payload.as_slice()).unwrap();
        if e.role == v1::ChatRole::Agent as i32 && want(&e.text) {
            return e.text;
        }
    }
}

/// ACP agents get the mesh: the node passes the agent's own mesh server through the bridge into ACP `session/new`, the agent starts it
/// (`agora mesh stdio --socket ...`, a real MCP conversation), sees the other agents and is seen by them, and its `agent_start` waits for
/// the user's decision like any agent's.
#[test]
fn an_acp_agent_uses_the_mesh_through_its_own_server() {
    let Some((home, node)) = node_with_acp("mesh") else {
        return;
    };
    let first = started_agent(&home);
    let second = started_agent(&home);

    let said = answer(&home, &first, "mcp-servers", |t| t.starts_with('['));
    let servers: serde_json::Value = serde_json::from_str(&said).unwrap();
    assert_eq!(servers.as_array().map(Vec::len), Some(1), "{servers}");
    assert_eq!(servers[0]["name"], "agora-mesh");
    assert_eq!(
        std::fs::canonicalize(servers[0]["command"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(AGORA).unwrap(),
        "the node's own program"
    );
    let args: Vec<&str> = servers[0]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert_eq!(args[..3], ["mesh", "stdio", "--socket"]);
    assert!(
        args[3].ends_with(&format!("{first}.sock")),
        "its own socket: {args:?}"
    );
    assert_eq!(servers[0]["env"], serde_json::json!([]));

    // Each sees the other through its own mesh server.
    let list = answer(&home, &second, "mcp agents_list {}", |t| {
        t.contains("agent_id")
    });
    assert!(
        list.contains(&format!("\"agent_id\":\"{first}\""))
            && list.contains("\"kind\":\"acp-fake\""),
        "{list}"
    );

    // A start it asks for waits for the user (no trusted device here, so nobody can answer it yet); while it waits, a second one is
    // refused as `pending`: the first reached the approval queue.
    let ask = r#"mcp agent_start {"kind":"acp-fake","prompt":"hello"}"#;
    let first_try = answer(&home, &first, ask, |t| {
        !t.starts_with('[') && !t.contains("agent_id")
    });
    assert_eq!(first_try, "waiting");
    let second_try = answer(&home, &first, ask, |t| t.starts_with("error"));
    assert!(
        second_try.contains("mesh.helper_limit") && second_try.contains("pending"),
        "{second_try}"
    );
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

// ---- ADR-0090: a structured agent's conversation, through a real node, the bridge and the fake ACP agent ----

/// One subscription to an agent's conversation, keeping every version of every entry it saw, in the order they arrived.
struct Chat {
    c: Client,
    seen: Vec<v1::ChatEntry>,
}

impl Chat {
    fn open(home: &Path, agent: &str) -> Self {
        let mut c = Client::connect(&home.join("node.sock")).unwrap();
        c.call_on(
            3,
            Cmd::ChatSubscribe(v1::ChatSubscribe {
                agent_id: agent.into(),
            }),
        )
        .unwrap()
        .expect("subscribed");
        Self {
            c,
            seen: Vec::new(),
        }
    }

    fn pump(&mut self) {
        let Some(f) = self.c.read_frame(Some(Duration::from_millis(100))).unwrap() else {
            return;
        };
        if f.stream_id != 3 {
            return;
        }
        match f.kind {
            KIND_CHAT_SNAPSHOT => self.seen.extend(
                v1::ChatSnapshot::decode(f.payload.as_slice())
                    .unwrap()
                    .entries,
            ),
            KIND_CHAT_ENTRY => self
                .seen
                .push(v1::ChatEntry::decode(f.payload.as_slice()).unwrap()),
            _ => {}
        }
    }

    /// The newest version of each entry, in entry order.
    fn latest(&self) -> Vec<v1::ChatEntry> {
        let mut by_id: std::collections::BTreeMap<u64, v1::ChatEntry> = Default::default();
        for e in &self.seen {
            by_id.insert(e.entry_id, e.clone());
        }
        by_id.into_values().collect()
    }

    fn wait(&mut self, what: &str, ok: impl Fn(&Chat) -> bool) {
        let end = Instant::now() + Duration::from_secs(30);
        while !ok(self) {
            assert!(
                Instant::now() < end,
                "never saw {what}: {:#?}",
                self.latest()
            );
            self.pump();
        }
    }
}

type Reply = Result<v1::AgentPromptResult, v1::ErrorInfo>;

/// `AgentPrompt` to an adapter agent: what the node answered (`AgentPromptResult`, or its error).
fn prompt(home: &Path, agent: &str, text: &str) -> Reply {
    let mut c = Client::connect(&home.join("node.sock")).unwrap();
    c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: agent.into(),
        text: text.into(),
    }))
    .unwrap()
    .map(|ok| match ok.result {
        Some(v1::command_ok::Result::AgentPrompt(r)) => r,
        other => panic!("not an AgentPromptResult: {other:?}"),
    })
}

fn command(home: &Path, cmd: Cmd) -> Result<v1::CommandOk, v1::ErrorInfo> {
    Client::connect(&home.join("node.sock"))
        .unwrap()
        .call(cmd)
        .unwrap()
}

fn param(e: &v1::ErrorInfo, k: &str) -> Option<v1::param_value::Value> {
    e.params.get(k).and_then(|p| p.value.clone())
}

fn says(chat: &Chat, text: &str) -> bool {
    chat.latest().iter().any(|e| {
        e.role == v1::ChatRole::Agent as i32
            && e.kind == v1::ChatKind::Text as i32
            && e.text == text
    })
}

/// Thinking, a tool call (one entry that changes in place, with status, files, output and a duration the node measured), a plan with
/// step status (one entry, replaced) and the answer: the conversation of a structured agent is complete.
#[test]
fn a_structured_agents_conversation_has_thinking_tool_status_and_plan_steps() {
    let Some((home, node)) = node_with_acp("rich") else {
        return;
    };
    let id = started_agent(&home);
    let mut chat = Chat::open(&home, &id);
    let sent = prompt(&home, &id, "rich").unwrap();
    assert!(!sent.queued && sent.queue_len == 0, "{sent:?}");
    chat.wait("the end of the turn", |c| says(c, "done"));
    // A version held back inside the 150 ms window is sent when the window is over, which can be after the message that followed it.
    chat.wait("the last version of the plan", |c| {
        c.latest().iter().any(|e| {
            e.kind == v1::ChatKind::Plan as i32
                && e.plan_steps
                    .first()
                    .is_some_and(|s| s.status == v1::ToolStatus::Completed as i32)
        })
    });

    let all = chat.latest();
    let thought = all
        .iter()
        .find(|e| e.kind == v1::ChatKind::Thought as i32)
        .expect("a thought");
    assert_eq!(thought.text, "thinking hard", "the chunks are joined");
    // One tool entry, however many times it changed; the versions the subscriber saw share its id.
    let tools: Vec<&v1::ChatEntry> = all
        .iter()
        .filter(|e| e.kind == v1::ChatKind::ToolCall as i32)
        .collect();
    assert_eq!(tools.len(), 1, "{tools:#?}");
    let tool = tools[0];
    let versions: Vec<&v1::ChatEntry> = chat.seen.iter().filter(|e| e.call_id == "t3").collect();
    assert!(
        versions.len() >= 3,
        "pending, in progress, completed: {versions:#?}"
    );
    assert!(versions.iter().all(|e| e.entry_id == tool.entry_id));
    assert_eq!(
        versions.first().unwrap().status,
        v1::ToolStatus::Pending as i32
    );
    assert_eq!(tool.status, v1::ToolStatus::Completed as i32);
    assert_eq!(tool.tool_kind, v1::ToolKind::Read as i32);
    assert_eq!(tool.output, "file text");
    assert_eq!(
        (tool.paths[0].path.as_str(), tool.paths[0].line),
        ("a.rs", 3)
    );
    assert_eq!(
        tool.text,
        "{\"tool\":\"read a.rs\",\"arguments\":{\"path\":\"a.rs\"}}"
    );
    // The node's clock measured it: the fake agent waited 400 ms between the first sight and the end.
    assert!(
        tool.started_ms > 0 && tool.ended_ms >= tool.started_ms + 300,
        "{tool:?}"
    );
    // The plan: one entry, replaced; its last version has the later status; old clients read the lines.
    let plans: Vec<&v1::ChatEntry> = all
        .iter()
        .filter(|e| e.kind == v1::ChatKind::Plan as i32)
        .collect();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].text, "read the code\nfix the bug");
    assert_eq!(
        plans[0].plan_steps[0].status,
        v1::ToolStatus::Completed as i32
    );
    assert_eq!(
        plans[0].plan_steps[1].status,
        v1::ToolStatus::InProgress as i32
    );
    assert_eq!(
        plans[0].plan_steps[0].priority,
        v1::PlanPriority::High as i32
    );
    // A subscriber that arrives later sees the same ring: one entry each, newest version.
    let mut late = Chat::open(&home, &id);
    late.wait("the snapshot", |c| !c.seen.is_empty());
    let snap = late.latest();
    assert_eq!(snap.iter().filter(|e| e.call_id == "t3").count(), 1);
    assert_eq!(
        snap.iter().find(|e| e.call_id == "t3").unwrap().status,
        v1::ToolStatus::Completed as i32
    );
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A prompt during a turn waits behind it, at most 8; cancelling the turn ends it and leaves the agent alive (only a stop ends the
/// agent); `drop_queue` throws the waiting prompts away.
#[test]
fn prompts_queue_behind_a_turn_and_cancelling_the_turn_leaves_the_agent_alive() {
    let Some((home, node)) = node_with_acp("queue") else {
        return;
    };
    let id = started_agent(&home);
    let mut chat = Chat::open(&home, &id);
    assert!(!prompt(&home, &id, "slow").unwrap().queued);
    chat.wait("the running call", |c| {
        c.latest()
            .iter()
            .any(|e| e.call_id == "t4" && e.status == v1::ToolStatus::InProgress as i32)
    });
    // A prompt that would wait is bounded (64 KiB); the same text would have been sent at once if the agent were free.
    let long = prompt(&home, &id, &"x".repeat(64 * 1024 + 1)).unwrap_err();
    assert_eq!(long.code, "protocol.invalid_message");
    // Eight wait, each answered with how many wait now; the ninth is refused with a stable code.
    for n in 1..=8u32 {
        let r = prompt(&home, &id, &format!("q{n}")).unwrap();
        assert!(r.queued && r.queue_len == n, "{n}: {r:?}");
    }
    let full = prompt(&home, &id, "q9").unwrap_err();
    assert_eq!(full.code, "agent.queue_full");
    assert_eq!(param(&full, "limit"), Some(v1::param_value::Value::Int(8)));
    chat.wait("eight queued entries", |c| {
        c.latest().iter().filter(|e| e.queued).count() == 8
    });
    assert!(chat
        .latest()
        .iter()
        .filter(|e| e.queued)
        .all(|e| e.role == v1::ChatRole::User as i32));

    // Cancel the turn and drop the queue: the agent returns to idle and is still there; the waiting prompts are gone with a notice.
    command(
        &home,
        Cmd::AgentCancelTurn(v1::AgentCancelTurn {
            agent_id: id.clone(),
            drop_queue: true,
        }),
    )
    .unwrap();
    wait_ready(&home, &id);
    chat.wait("the notice", |c| {
        c.latest().iter().any(|e| e.code == "adapter.queue_dropped")
    });
    let notice = chat
        .latest()
        .into_iter()
        .find(|e| e.code == "adapter.queue_dropped")
        .unwrap();
    assert_eq!(notice.params["count"], "8");
    let mut fresh = Chat::open(&home, &id);
    fresh.wait("the snapshot", |c| !c.seen.is_empty());
    assert!(
        fresh.latest().iter().all(|e| !e.queued),
        "no waiting prompt survives in the ring"
    );
    assert!(!says(&fresh, "q1"), "a dropped prompt was never sent");
    // The agent is alive and takes the next prompt.
    assert!(!prompt(&home, &id, "again").unwrap().queued);
    chat.wait("the agent answering again", |c| says(c, "again"));
    let (_, list) = agora(&home, &["agent", "list"]);
    assert!(
        list.lines()
            .any(|l| l.contains(&id) && !l.contains("stopped")),
        "{list}"
    );
    // A stop ends it.
    command(
        &home,
        Cmd::AgentStop(v1::AgentStop {
            agent_id: id.clone(),
            stop_helpers: false,
        }),
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, list) = agora(&home, &["agent", "list"]);
        if !list.lines().any(|l| l.contains(&id) && l.contains("idle")) {
            break;
        }
        assert!(Instant::now() < end, "{list}");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// Cancelling the turn without `drop_queue` keeps what waits: it is sent, in order, as the turns end.
#[test]
fn cancelling_a_turn_keeps_the_queue_unless_told_to_drop_it() {
    let Some((home, node)) = node_with_acp("queue-keep") else {
        return;
    };
    let id = started_agent(&home);
    let mut chat = Chat::open(&home, &id);
    assert!(!prompt(&home, &id, "slow").unwrap().queued);
    chat.wait("the running call", |c| {
        c.latest().iter().any(|e| e.call_id == "t4")
    });
    for t in ["k1", "k2"] {
        assert!(prompt(&home, &id, t).unwrap().queued);
    }
    command(
        &home,
        Cmd::AgentCancelTurn(v1::AgentCancelTurn {
            agent_id: id.clone(),
            drop_queue: false,
        }),
    )
    .unwrap();
    chat.wait("both answered", |c| says(c, "k1") && says(c, "k2"));
    let all = chat.latest();
    let at = |t: &str| {
        all.iter()
            .position(|e| e.role == v1::ChatRole::Agent as i32 && e.text == t)
            .unwrap()
    };
    assert!(at("k1") < at("k2"), "in order");
    assert!(
        all.iter().all(|e| !e.queued),
        "nothing is still marked as waiting: {all:#?}"
    );
    // Their entries are the user's own prompts, sent and no longer queued.
    assert!(all
        .iter()
        .any(|e| e.role == v1::ChatRole::User as i32 && e.text == "k1"));
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// A permission request of the agent is in the conversation as an entry that shows it pending; cancelling the turn answers it
/// cancelled, from the node's own approval state, and the agent lives on.
#[test]
fn cancelling_a_turn_cancels_its_permission_entry() {
    let Some((home, node)) = node_with_acp("perm-cancel") else {
        return;
    };
    let id = started_agent(&home);
    let mut chat = Chat::open(&home, &id);
    assert!(!prompt(&home, &id, "ask").unwrap().queued);
    chat.wait("a pending permission", |c| {
        c.latest().iter().any(|e| {
            e.kind == v1::ChatKind::Permission as i32
                && e.permission_outcome == v1::PermissionOutcome::Pending as i32
        })
    });
    let entry = chat
        .latest()
        .into_iter()
        .find(|e| e.kind == v1::ChatKind::Permission as i32)
        .unwrap();
    assert!(
        entry.approval_id.starts_with("ap-"),
        "the node's own id, not the bridge's `acp-N`: {entry:?}"
    );
    assert_eq!(entry.text, "run_command");
    command(
        &home,
        Cmd::AgentCancelTurn(v1::AgentCancelTurn {
            agent_id: id.clone(),
            drop_queue: false,
        }),
    )
    .unwrap();
    chat.wait("cancelled", |c| {
        c.latest().iter().any(|e| {
            e.approval_id == entry.approval_id
                && e.permission_outcome == v1::PermissionOutcome::Cancelled as i32
        })
    });
    wait_ready(&home, &id);
    // Cancelling is for agents with a turn: an unknown agent is not found.
    let err = command(
        &home,
        Cmd::AgentCancelTurn(v1::AgentCancelTurn {
            agent_id: "agent-404".into(),
            drop_queue: false,
        }),
    )
    .unwrap_err();
    assert_eq!(err.code, "agent.not_found");
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}

/// `AgentSetConfig`: the id and the value are checked against what the agent reported; an option that skips permissions (or that the
/// node cannot tell from one) follows the node's `skip_permissions` policy, whatever category the agent gave it.
#[test]
fn a_config_option_is_set_checked_and_held_to_the_skip_permissions_policy() {
    for (policy, allowed) in [
        ("skip_permissions = forbid\n", false),
        ("skip_permissions = allow\n", true),
    ] {
        let tag = if allowed {
            "config-allow"
        } else {
            "config-forbid"
        };
        let Some((home, node)) = node_with_acp_policy(tag, Some(policy)) else {
            return;
        };
        let id = started_agent_of(&home, "acp-config");
        let set = |config_id: &str, value: &str| {
            command(
                &home,
                Cmd::AgentSetConfig(v1::AgentSetConfig {
                    agent_id: id.clone(),
                    config_id: config_id.into(),
                    value: value.into(),
                }),
            )
        };
        // The report arrives just after the start.
        let end = Instant::now() + Duration::from_secs(15);
        while set("effort", "low").is_err() {
            assert!(Instant::now() < end, "the option was never accepted");
            std::thread::sleep(Duration::from_millis(100));
        }
        // A reasoning level, a safe mode and an unrelated boolean are never held to the policy.
        assert!(set("effort", "high").is_ok());
        assert!(set("approvals", "plan").is_ok());
        assert!(set("extended", "true").is_ok());
        // Not reported by the agent: a stable error with the id that is not known.
        let unknown = set("nope", "x").unwrap_err();
        assert_eq!(unknown.code, "agent.option_unknown");
        assert_eq!(
            param(&unknown, "id"),
            Some(v1::param_value::Value::Str("nope".into()))
        );
        let unknown = set("effort", "extreme").unwrap_err();
        assert_eq!(unknown.code, "agent.option_unknown");
        assert_eq!(
            param(&unknown, "id"),
            Some(v1::param_value::Value::Str("extreme".into()))
        );
        assert_eq!(
            set("extended", "yes").unwrap_err().code,
            "agent.option_unknown"
        );
        assert_eq!(set("effort", "").unwrap_err().code, "agent.option_unknown");
        // `turbo` is in no list of bad names; the node cannot tell it asks, so it is held to the policy like `bypass`. The
        // boolean about approvals too.
        // Turning auto-approve OFF is protection: allowed under any policy. Turning it ON is refused.
        assert!(set("auto_approve", "false").is_ok());
        for (config_id, value) in [("approvals", "turbo"), ("auto_approve", "true")] {
            let r = set(config_id, value);
            if allowed {
                // Allowed by the policy; not applied, asked about (ADR-0090 Amendment A).
                let err = r.unwrap_err();
                assert_eq!(err.code, "agent.approval_needed", "{config_id}={value}");
                assert!(param(&err, "approval_id").is_some());
            } else {
                let err = r.unwrap_err();
                assert_eq!(err.code, "agent.option_forbidden", "{config_id}={value}");
                assert_eq!(
                    param(&err, "policy"),
                    Some(v1::param_value::Value::Str("forbid".into()))
                );
                assert_eq!(
                    param(&err, "option"),
                    Some(v1::param_value::Value::Str(config_id.into()))
                );
            }
        }
        // An agent that reported no options has nothing to set.
        let plain = started_agent(&home);
        let r = command(
            &home,
            Cmd::AgentSetConfig(v1::AgentSetConfig {
                agent_id: plain,
                config_id: "effort".into(),
                value: "low".into(),
            }),
        );
        assert_eq!(r.unwrap_err().code, "agent.option_unknown");
        drop(node);
        let _ = std::fs::remove_dir_all(&home);
    }
}

/// The last update of an entry is always delivered: updates that come inside one 150 ms window, with nothing after them, still go out
/// when the window is over (the node's tick sends what it held back), for a tool call and for the plan.
#[test]
fn the_last_update_inside_a_coalescing_window_is_still_delivered() {
    let Some((home, node)) = node_with_acp("burst") else {
        return;
    };
    let id = started_agent(&home);
    let mut chat = Chat::open(&home, &id);
    assert!(!prompt(&home, &id, "burst").unwrap().queued);
    chat.wait("the end of the turn", |c| says(c, "burst-done"));
    // Nothing follows the burst; the final state must still arrive within the window and a tick or two.
    let begun = Instant::now();
    chat.wait("the last tool output and plan status", |c| {
        let all = c.latest();
        all.iter().any(|e| e.call_id == "t5" && e.output == "three")
            && all.iter().any(|e| {
                e.kind == v1::ChatKind::Plan as i32
                    && e.text == "only step"
                    && e.plan_steps[0].status == v1::ToolStatus::Completed as i32
            })
    });
    assert!(
        begun.elapsed() < Duration::from_secs(5),
        "{:?}",
        begun.elapsed()
    );
    // What a late subscriber is given is the same final state.
    let mut late = Chat::open(&home, &id);
    late.wait("the snapshot", |c| !c.seen.is_empty());
    assert!(late
        .latest()
        .iter()
        .any(|e| e.call_id == "t5" && e.output == "three"));
    drop(node);
    let _ = std::fs::remove_dir_all(&home);
}
